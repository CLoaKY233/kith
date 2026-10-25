use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use crate::Error;
use crate::frame::{Decoder, Event, OP_BINARY, OP_CLOSE, OP_PING, OP_PONG, encode};
use crate::stream::Stream;

/// Idle time after which a link pings its peer; three silent intervals end the link.
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);
/// Largest datagram a link carries: a 64 KiB packet plus room for relay framing.
pub const MAX_DATAGRAM: usize = 1 << 17;
const SILENT_INTERVALS_BEFORE_DEAD: u32 = 3;
const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug)]
struct Writer {
    half: WriteHalf<Stream>,
    frame: Vec<u8>,
    /// Client masks only need to be unpredictable to a proxy, not secret (§10.3), and
    /// TLS hides them anyway, so a seeded xorshift avoids a syscall per frame.
    mask_state: Option<u64>,
}

impl Writer {
    async fn write(&mut self, opcode: u8, payload: &[u8]) -> Result<(), Error> {
        let mask = self.mask_state.as_mut().map(|state| {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            let [a, b, c, d, ..] = state.to_le_bytes();
            [a, b, c, d]
        });
        self.frame.clear();
        encode(&mut self.frame, true, opcode, payload, mask);
        self.half.write_all(&self.frame).await?;
        self.half.flush().await?;
        Ok(())
    }
}

/// Cloneable sending half of a [`WsLink`].
#[derive(Debug, Clone)]
pub struct WsSender {
    writer: Arc<Mutex<Writer>>,
}

impl WsSender {
    /// Sends one datagram as one binary message.
    pub async fn send(&self, datagram: &[u8]) -> Result<(), Error> {
        if datagram.len() > MAX_DATAGRAM {
            return Err(Error::TooLarge);
        }
        self.writer.lock().await.write(OP_BINARY, datagram).await
    }

    /// Sends a close frame and shuts the stream down.
    pub async fn close(&self) -> Result<(), Error> {
        let mut writer = self.writer.lock().await;
        writer.write(OP_CLOSE, &1000u16.to_be_bytes()).await?;
        writer.half.shutdown().await?;
        Ok(())
    }

    async fn control(&self, opcode: u8, payload: &[u8]) -> Result<(), Error> {
        self.writer.lock().await.write(opcode, payload).await
    }
}

/// An established WebSocket link carrying one datagram per binary message.
#[derive(Debug)]
pub struct WsLink {
    reader: ReadHalf<Stream>,
    sender: WsSender,
    decoder: Decoder,
    read_buffer: Box<[u8]>,
}

impl WsLink {
    pub(crate) fn new(stream: Stream, is_client: bool, leftover: &[u8]) -> Result<Self, Error> {
        let mask_state = if is_client {
            let mut seed = [0u8; 8];
            kith_crypto::fill_random(&mut seed)?;
            Some(u64::from_le_bytes(seed) | 1)
        } else {
            None
        };
        let (reader, half) = tokio::io::split(stream);
        let mut decoder = Decoder::new(!is_client, MAX_DATAGRAM);
        decoder.push(leftover);
        Ok(Self {
            reader,
            sender: WsSender {
                writer: Arc::new(Mutex::new(Writer {
                    half,
                    frame: Vec::new(),
                    mask_state,
                })),
            },
            decoder,
            read_buffer: vec![0; READ_CHUNK].into_boxed_slice(),
        })
    }

    /// A handle for sending concurrently with [`Self::recv`].
    pub fn sender(&self) -> WsSender {
        self.sender.clone()
    }

    /// Sends one datagram.
    pub async fn send(&self, datagram: &[u8]) -> Result<(), Error> {
        self.sender.send(datagram).await
    }

    /// Receives the next datagram, answering pings and sending keepalives meanwhile.
    ///
    /// Not cancel-safe: dropping the future mid-pong can corrupt the stream.
    pub async fn recv(&mut self) -> Result<Vec<u8>, Error> {
        let mut silent_intervals = 0;
        loop {
            while let Some(event) = self.decoder.next_event()? {
                match event {
                    Event::Binary(datagram) => return Ok(datagram),
                    Event::Ping(payload) => self.sender.control(OP_PONG, &payload).await?,
                    Event::Pong => {}
                    Event::Close => {
                        // The peer may already be gone; the link is closed either way.
                        self.sender.close().await.ok();
                        return Err(Error::Closed);
                    }
                }
            }
            match tokio::time::timeout(KEEPALIVE_INTERVAL, self.reader.read(&mut self.read_buffer))
                .await
            {
                Ok(Ok(0)) => return Err(Error::Closed),
                Ok(Ok(read)) => {
                    silent_intervals = 0;
                    self.decoder
                        .push(self.read_buffer.get(..read).unwrap_or_default());
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => {
                    silent_intervals += 1;
                    if silent_intervals >= SILENT_INTERVALS_BEFORE_DEAD {
                        return Err(Error::Timeout);
                    }
                    self.sender.control(OP_PING, &[]).await?;
                }
            }
        }
    }
}
