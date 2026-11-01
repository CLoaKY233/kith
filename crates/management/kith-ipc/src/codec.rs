//! Envelope and frame encoding over any async byte stream.

use kith_wire::{FrameDecoder, WireError, envelope, frame, registry::IPC_MESSAGE};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{IpcError, PROTOCOL_VERSION};

#[cfg(test)]
pub(crate) fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    Ok(framed(&envelope::encode(
        IPC_MESSAGE,
        u64::from(PROTOCOL_VERSION),
        message,
    )?))
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len().saturating_add(4));
    frame::encode(body, &mut out);
    out
}

/// Decodes one frame payload. A different envelope version is a version mismatch.
pub(crate) fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, IpcError> {
    let (header, body) = envelope::decode_header(payload)?;
    if header.message_type != IPC_MESSAGE {
        return Err(IpcError::Unexpected);
    }
    if header.version != u64::from(PROTOCOL_VERSION) {
        return Err(IpcError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: u32::try_from(header.version).unwrap_or(u32::MAX),
        });
    }
    Ok(envelope::decode_body(body)?)
}

#[derive(Debug)]
pub(crate) struct Framed<S> {
    stream: S,
    decoder: FrameDecoder,
    /// The largest envelope the other side accepts.
    max_send: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Framed<S> {
    /// Reads frames up to `max_recv` bytes and refuses to send any over `max_send`, the
    /// other side's limit, so an oversized message fails here rather than after the peer
    /// drops the connection.
    pub(crate) fn new(stream: S, max_recv: usize, max_send: usize) -> Self {
        Self {
            stream,
            decoder: FrameDecoder::new(max_recv),
            max_send,
        }
    }

    pub(crate) async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), IpcError> {
        let body = envelope::encode(IPC_MESSAGE, u64::from(PROTOCOL_VERSION), message)?;
        if body.len() > self.max_send {
            return Err(IpcError::Wire(WireError::TooLarge {
                len: u64::try_from(body.len()).unwrap_or(u64::MAX),
                max: u64::try_from(self.max_send).unwrap_or(u64::MAX),
            }));
        }
        let bytes = framed(&body);
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// The next message, or `None` on a clean end of stream.
    pub(crate) async fn recv<T: DeserializeOwned>(&mut self) -> Result<Option<T>, IpcError> {
        let mut chunk = [0_u8; 4096];
        loop {
            if let Some(payload) = self.decoder.next_frame()? {
                return decode(&payload).map(Some);
            }
            let read = self.stream.read(&mut chunk).await?;
            if read == 0 {
                return if self.decoder.buffered() == 0 {
                    Ok(None)
                } else {
                    Err(IpcError::Closed)
                };
            }
            self.decoder.push(chunk.get(..read).unwrap_or_default());
        }
    }
}
