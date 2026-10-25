//! Envelope and frame encoding over any async byte stream.

use kith_wire::{FrameDecoder, envelope, frame, registry::IPC_MESSAGE};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{IpcError, MAX_FRAME_LEN, PROTOCOL_VERSION};

pub(crate) fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    let body = envelope::encode(IPC_MESSAGE, u64::from(PROTOCOL_VERSION), message)?;
    let mut out = Vec::with_capacity(body.len().saturating_add(4));
    frame::encode(&body, &mut out);
    Ok(out)
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
}

impl<S: AsyncRead + AsyncWrite + Unpin> Framed<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            decoder: FrameDecoder::new(MAX_FRAME_LEN),
        }
    }

    pub(crate) async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), IpcError> {
        let bytes = encode(message)?;
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
