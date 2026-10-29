use std::net::SocketAddr;

use kith_ledger::{JoinProof, MAX_NAME_LEN, MAX_SIGNED_OP_LEN};
use kith_sync::MAX_SYNC_MESSAGE_LEN;
use kith_types::{NetworkId, NodeId};
use kith_wire::registry::CHANNEL_MESSAGE;
use kith_wire::{FrameDecoder, WireError, envelope, frame};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{ChannelError, MAX_ENDPOINT_ADDRS, MAX_ENDPOINT_NODES};

const CHANNEL_VERSION: u64 = 1;

const MAX_NETWORKS: usize = 256;

const MAX_REASON_LEN: usize = 256;

pub(crate) const MAX_MESSAGE_LEN: usize = MAX_SYNC_MESSAGE_LEN + 1024;

const _: () = assert!(MAX_MESSAGE_LEN <= kith_wire::MAX_MESSAGE_LEN);

const READ_CHUNK: usize = 16 * 1024;

pub(crate) type Entries = Vec<(NodeId, Vec<SocketAddr>)>;

/// Whether a plain WireGuard member, such as a phone, is connected to its gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// Whether the gateway heard from it recently.
    pub online: bool,
    /// When the gateway last heard from it, in Unix seconds; 0 if never.
    pub last_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Message {
    Hello {
        networks: Vec<NetworkId>,
    },
    JoinRequest {
        network: NetworkId,
        proof: Box<JoinProof>,
    },
    JoinAccepted {
        genesis: Vec<u8>,
    },
    JoinRejected {
        reason: String,
    },
    Sync {
        network: NetworkId,
        bytes: Vec<u8>,
    },
    Endpoints {
        network: NetworkId,
        entries: Entries,
    },
    Presence {
        network: NetworkId,
        entries: Vec<(NodeId, Presence)>,
    },
}

impl Message {
    pub(crate) fn rejection(reason: &str) -> Self {
        let end = reason.floor_char_boundary(MAX_REASON_LEN);
        Self::JoinRejected {
            reason: reason.get(..end).unwrap_or_default().to_owned(),
        }
    }

    pub(crate) fn to_frame(&self) -> Result<Vec<u8>, ChannelError> {
        self.validate()?;
        let payload = envelope::encode(CHANNEL_MESSAGE, CHANNEL_VERSION, self)?;
        if payload.len() > MAX_MESSAGE_LEN {
            return Err(ChannelError::LimitExceeded);
        }
        let mut out = Vec::new();
        frame::encode(&payload, &mut out);
        Ok(out)
    }

    pub(crate) fn decode(payload: &[u8]) -> Result<Self, ChannelError> {
        let (version, message): (u64, Self) =
            envelope::decode(payload, CHANNEL_MESSAGE, CHANNEL_VERSION)?;
        if version != CHANNEL_VERSION {
            return Err(WireError::UnsupportedVersion {
                found: version,
                supported: CHANNEL_VERSION,
            }
            .into());
        }
        message.validate()?;
        Ok(message)
    }

    fn validate(&self) -> Result<(), ChannelError> {
        match self {
            Self::Hello { networks } => within(networks.len(), MAX_NETWORKS),
            Self::JoinRequest { proof, .. } => within(proof.name.len(), MAX_NAME_LEN),
            Self::JoinAccepted { genesis } => within(genesis.len(), MAX_SIGNED_OP_LEN),
            Self::JoinRejected { reason } => within(reason.len(), MAX_REASON_LEN),
            Self::Sync { bytes, .. } => within(bytes.len(), MAX_SYNC_MESSAGE_LEN),
            Self::Endpoints { entries, .. } => {
                within(entries.len(), MAX_ENDPOINT_NODES)?;
                entries
                    .iter()
                    .try_for_each(|(_, addrs)| within(addrs.len(), MAX_ENDPOINT_ADDRS))
            }
            Self::Presence { entries, .. } => within(entries.len(), MAX_ENDPOINT_NODES),
        }
    }
}

fn within(len: usize, max: usize) -> Result<(), ChannelError> {
    if len <= max {
        Ok(())
    } else {
        Err(ChannelError::LimitExceeded)
    }
}

pub(crate) struct FrameReader {
    decoder: FrameDecoder,
    buffer: Vec<u8>,
    pending_limit: usize,
}

impl FrameReader {
    pub(crate) fn new() -> Self {
        Self {
            decoder: FrameDecoder::new(MAX_MESSAGE_LEN),
            buffer: vec![0; READ_CHUNK],
            pending_limit: usize::MAX,
        }
    }

    pub(crate) const fn limit_pending(&mut self, limit: usize) {
        self.pending_limit = limit;
    }

    pub(crate) async fn next<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<Message, ChannelError> {
        loop {
            if let Some(payload) = self.decoder.next_frame()? {
                return Message::decode(&payload);
            }
            if self.decoder.buffered() > self.pending_limit {
                return Err(ChannelError::LimitExceeded);
            }
            let read = reader.read(&mut self.buffer).await?;
            if read == 0 {
                return Err(ChannelError::Closed);
            }
            self.decoder
                .push(self.buffer.get(..read).unwrap_or_default());
        }
    }
}

pub(crate) async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Message,
) -> Result<(), ChannelError> {
    writer.write_all(&message.to_frame()?).await?;
    writer.flush().await?;
    Ok(())
}
