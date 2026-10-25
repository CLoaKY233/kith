//! Many peers behind one identity, with datagram routing.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::fmt;

use crate::WgError;
use crate::cookie::{CookieChecker, Incoming};
use crate::messages::{INITIATION_MAC1_OFFSET, MIN_OUTPUT_LEN, Message, RESPONSE_MAC1_OFFSET};
use crate::noise::{StaticIdentity, begin_initiation};
use crate::peer::{Decapsulated, Peer, PeerConfig, verify_handshake};
use crate::random::{RandomSource, random_u32};
use crate::time::Now;

const INDEX_PREFIX_MASK: u32 = !0xff;
const PREFIX_ATTEMPTS: usize = 16;

/// Handle of a peer inside a [`Device`]: the upper 24 bits of its session indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(u32);

/// What [`Device::decapsulate`] did, and for which peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceOutput {
    /// The peer the datagram belonged to; `None` for a cookie reply sent before routing.
    pub peer: Option<PeerId>,
    /// The result, with any output in the caller's buffer.
    pub result: Decapsulated,
}

/// Owns an identity and its peers, and routes incoming datagrams to them.
pub struct Device {
    identity: Arc<StaticIdentity>,
    peers: BTreeMap<PeerId, Peer>,
    by_public_key: BTreeMap<[u8; 32], PeerId>,
    checker: CookieChecker,
    random: Box<dyn RandomSource + Send>,
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("identity", &self.identity)
            .field("peers", &self.peers.len())
            .finish_non_exhaustive()
    }
}

impl Device {
    /// A device with the given private key and a cookie checker shared by all peers.
    #[must_use]
    pub fn new(
        secret: [u8; 32],
        checker: CookieChecker,
        random: Box<dyn RandomSource + Send>,
    ) -> Self {
        Self {
            identity: Arc::new(StaticIdentity::from_secret_bytes(secret)),
            peers: BTreeMap::new(),
            by_public_key: BTreeMap::new(),
            checker,
            random,
        }
    }

    /// Our public key.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.identity.public_key()
    }

    /// Adds a peer, which draws its own randomness from `random`.
    pub fn add_peer(
        &mut self,
        config: &PeerConfig,
        random: Box<dyn RandomSource + Send>,
    ) -> Result<PeerId, WgError> {
        if self.by_public_key.contains_key(&config.public_key()) {
            return Err(WgError::PeerRejected);
        }
        let mut id = None;
        for _ in 0..PREFIX_ATTEMPTS {
            let candidate = PeerId(random_u32(self.random.as_mut())? & INDEX_PREFIX_MASK);
            if !self.peers.contains_key(&candidate) {
                id = Some(candidate);
                break;
            }
        }
        let id = id.ok_or(WgError::PeerRejected)?;
        let public_key = config.public_key();
        let peer = Peer::with_index_prefix(Arc::clone(&self.identity), config, random, id.0)?;
        debug_assert_eq!(
            peer.local_index_prefix(),
            id.0,
            "peer uses the assigned prefix"
        );
        self.peers.insert(id, peer);
        self.by_public_key.insert(public_key, id);
        Ok(id)
    }

    /// Removes a peer and erases its keys.
    pub fn remove_peer(&mut self, id: PeerId) -> Option<Peer> {
        let peer = self.peers.remove(&id)?;
        self.by_public_key.remove(&peer.public_key());
        Some(peer)
    }

    /// The peer with this handle.
    pub fn peer_mut(&mut self, id: PeerId) -> Option<&mut Peer> {
        self.peers.get_mut(&id)
    }

    /// The peer with this public key.
    pub fn peer_by_public_key(&self, public_key: &[u8; 32]) -> Option<PeerId> {
        self.by_public_key.get(public_key).copied()
    }

    /// All peers, for driving timers.
    pub fn peers_mut(&mut self) -> impl Iterator<Item = (PeerId, &mut Peer)> {
        self.peers.iter_mut().map(|(id, peer)| (*id, peer))
    }

    /// Routes and processes one datagram. `source` encodes the sender's address for cookies.
    pub fn decapsulate(
        &mut self,
        now: Now,
        source: &[u8],
        datagram: &[u8],
        out: &mut [u8],
    ) -> Result<DeviceOutput, WgError> {
        if out.len() < MIN_OUTPUT_LEN.max(datagram.len()) {
            return Err(WgError::BufferTooSmall);
        }
        let (id, result) = match Message::parse(datagram)? {
            Message::Initiation(message) => {
                if let Some(result) = self.verify(
                    now,
                    &Incoming {
                        source,
                        datagram,
                        mac1_offset: INITIATION_MAC1_OFFSET,
                        sender: message.sender,
                    },
                    out,
                )? {
                    return Ok(DeviceOutput { peer: None, result });
                }
                let half = begin_initiation(&self.identity, &message)?;
                let id = self
                    .peer_by_public_key(&half.peer_public)
                    .ok_or(WgError::UnknownPeer)?;
                (id, self.routed(id)?.handle_initiation(now, half, out)?)
            }
            Message::Response(message) => {
                if let Some(result) = self.verify(
                    now,
                    &Incoming {
                        source,
                        datagram,
                        mac1_offset: RESPONSE_MAC1_OFFSET,
                        sender: message.sender,
                    },
                    out,
                )? {
                    return Ok(DeviceOutput { peer: None, result });
                }
                let id = Self::route(message.receiver);
                (id, self.routed(id)?.handle_response(now, &message, out)?)
            }
            Message::CookieReply(reply) => {
                let id = Self::route(reply.receiver);
                (id, self.routed(id)?.handle_cookie_reply(now, &reply)?)
            }
            Message::Transport(message) => {
                let id = Self::route(message.receiver);
                (id, self.routed(id)?.handle_transport(now, &message, out)?)
            }
        };
        Ok(DeviceOutput {
            peer: Some(id),
            result,
        })
    }

    fn routed(&mut self, id: PeerId) -> Result<&mut Peer, WgError> {
        self.peers.get_mut(&id).ok_or(WgError::UnknownIndex)
    }

    fn verify(
        &mut self,
        now: Now,
        incoming: &Incoming<'_>,
        out: &mut [u8],
    ) -> Result<Option<Decapsulated>, WgError> {
        verify_handshake(
            &mut self.checker,
            self.random.as_mut(),
            &self.identity,
            now.monotonic,
            incoming,
            out,
        )
    }

    const fn route(receiver: u32) -> PeerId {
        PeerId(receiver & INDEX_PREFIX_MASK)
    }
}
