//! The relay client state machine.
//!
//! Sans-IO: the driver decodes frames from the stream and passes them to
//! [`RelayClient::handle_frame`], writes whatever [`RelayClient::poll_transmit`] returns, and
//! reacts to [`RelayClient::poll_event`]. Packets sent before the relay accepts us are held
//! (up to [`MAX_PENDING_BYTES`]) and released in order on acceptance.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::borrow::Borrow;

use kith_crypto::SigningKey;
use kith_types::NodeId;

use crate::auth::sign_challenge;
use crate::frame::{Frame, HealthProblem, MAX_PAYLOAD_LEN, PROTOCOL_VERSION, RejectReason};

/// Packet bytes the client holds while waiting for the relay to accept it.
pub const MAX_PENDING_BYTES: usize = 256 * 1024;

/// Something the driver should know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The relay accepted us; packets now flow.
    Connected,
    /// The relay refused us. The connection is finished.
    Rejected {
        /// Why.
        reason: RejectReason,
    },
    /// `src` (authenticated by the relay) sent us `payload`.
    Packet {
        /// The sender.
        src: NodeId,
        /// The opaque, encrypted packet.
        payload: Vec<u8>,
    },
    /// `node` is not connected to this relay.
    PeerGone {
        /// The unreachable node.
        node: NodeId,
    },
    /// The relay answered one of our pings.
    Pong {
        /// The data we sent.
        data: [u8; 8],
    },
    /// The relay reported its health.
    Health {
        /// `None` when healthy.
        problem: Option<HealthProblem>,
    },
}

/// The relay misbehaved. The driver should close the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The relay speaks a protocol version we do not.
    #[error("relay speaks protocol version {0}")]
    UnsupportedVersion(u16),
    /// The relay sent a frame that is not valid in the current state.
    #[error("unexpected relay frame of type {frame_type:#04x}")]
    UnexpectedFrame {
        /// The type byte of the offending frame.
        frame_type: u8,
    },
}

/// [`RelayClient::send`] could not queue a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SendError {
    /// The payload exceeds [`MAX_PAYLOAD_LEN`].
    #[error("payload of {0} bytes exceeds the relay limit")]
    TooLarge(usize),
    /// Not yet accepted and [`MAX_PENDING_BYTES`] are already held; the packet was dropped.
    #[error("pending queue full; packet dropped")]
    QueueFull,
    /// The connection was rejected or failed.
    #[error("relay connection is closed")]
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitingChallenge,
    AwaitingAccept,
    Connected,
    Closed,
}

/// The relay client. `K` is anything that borrows the device's [`SigningKey`]: the key
/// itself, a reference, or a shared pointer.
#[derive(Debug)]
pub struct RelayClient<K> {
    key: K,
    relay: NodeId,
    state: State,
    transmit: VecDeque<Frame>,
    pending: VecDeque<Frame>,
    pending_bytes: usize,
    events: VecDeque<Event>,
    packets_dropped: u64,
}

impl<K: Borrow<SigningKey>> RelayClient<K> {
    /// A client that will authenticate with `key` to the relay whose identity is `relay`.
    pub const fn new(key: K, relay: NodeId) -> Self {
        Self {
            key,
            relay,
            state: State::AwaitingChallenge,
            transmit: VecDeque::new(),
            pending: VecDeque::new(),
            pending_bytes: 0,
            events: VecDeque::new(),
            packets_dropped: 0,
        }
    }

    /// Processes a frame from the relay. On error the connection is closed.
    pub fn handle_frame(&mut self, frame: Frame) -> Result<(), ClientError> {
        let result = self.dispatch(frame);
        if result.is_err() {
            self.close();
        }
        result
    }

    fn dispatch(&mut self, frame: Frame) -> Result<(), ClientError> {
        match (self.state, frame) {
            (
                State::AwaitingChallenge,
                Frame::ServerChallenge {
                    protocol_version,
                    nonce,
                },
            ) => {
                if protocol_version != PROTOCOL_VERSION {
                    return Err(ClientError::UnsupportedVersion(protocol_version));
                }
                let key = self.key.borrow();
                let signature = sign_challenge(key, &nonce, &self.relay);
                self.transmit.push_back(Frame::ClientHello {
                    node: key.node_id(),
                    signature,
                });
                self.state = State::AwaitingAccept;
            }
            (State::AwaitingAccept, Frame::ServerAccept) => {
                self.state = State::Connected;
                self.transmit.append(&mut self.pending);
                self.pending_bytes = 0;
                self.events.push_back(Event::Connected);
            }
            (State::AwaitingAccept, Frame::ServerReject { reason }) => {
                self.close();
                self.events.push_back(Event::Rejected { reason });
            }
            (State::Connected, Frame::RecvPacket { src, payload }) => {
                self.events.push_back(Event::Packet { src, payload });
            }
            (State::Connected, Frame::PeerGone { node }) => {
                self.events.push_back(Event::PeerGone { node });
            }
            (State::Connected, Frame::Ping { data }) => {
                self.transmit.push_back(Frame::Pong { data });
            }
            (State::Connected, Frame::Pong { data }) => {
                self.events.push_back(Event::Pong { data });
            }
            (State::Connected, Frame::Health { problem }) => {
                self.events.push_back(Event::Health { problem });
            }
            (_, frame) => {
                return Err(ClientError::UnexpectedFrame {
                    frame_type: frame.frame_type(),
                });
            }
        }
        Ok(())
    }

    /// Queues `payload` for `dst`. Before acceptance the packet is held, and dropped once
    /// [`MAX_PENDING_BYTES`] are held. After acceptance the driver applies backpressure by
    /// not calling this while its stream is full.
    pub fn send(&mut self, dst: NodeId, payload: Vec<u8>) -> Result<(), SendError> {
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(SendError::TooLarge(payload.len()));
        }
        let frame = Frame::SendPacket { dst, payload };
        match self.state {
            State::Connected => self.transmit.push_back(frame),
            State::AwaitingChallenge | State::AwaitingAccept => {
                let held = self.pending_bytes.saturating_add(frame.encoded_len());
                if held > MAX_PENDING_BYTES {
                    self.packets_dropped = self.packets_dropped.saturating_add(1);
                    return Err(SendError::QueueFull);
                }
                self.pending_bytes = held;
                self.pending.push_back(frame);
            }
            State::Closed => return Err(SendError::Closed),
        }
        Ok(())
    }

    /// Queues a liveness probe; the answer arrives as [`Event::Pong`]. Ignored until
    /// connected.
    pub fn ping(&mut self, data: [u8; 8]) {
        if self.state == State::Connected {
            self.transmit.push_back(Frame::Ping { data });
        }
    }

    /// The next frame to write to the stream.
    pub fn poll_transmit(&mut self) -> Option<Frame> {
        self.transmit.pop_front()
    }

    /// The next event for the driver.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Whether the relay has accepted us and the connection is still up.
    pub fn is_connected(&self) -> bool {
        self.state == State::Connected
    }

    /// Packets dropped because the pending queue was full.
    pub const fn packets_dropped(&self) -> u64 {
        self.packets_dropped
    }

    fn close(&mut self) {
        self.state = State::Closed;
        self.pending.clear();
        self.pending_bytes = 0;
    }
}
