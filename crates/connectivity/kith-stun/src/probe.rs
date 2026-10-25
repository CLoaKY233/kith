//! A sans-IO STUN client that retransmits until answered or out of attempts.

use core::net::SocketAddr;
use core::time::Duration;

use crate::message::{REQUEST_LEN, TransactionId, encode_binding_request, parse_binding_response};

/// When each transmission is sent, measured from probe creation; the gaps double.
pub(crate) const RETRANSMIT_SCHEDULE: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_millis(100),
    Duration::from_millis(300),
    Duration::from_millis(700),
];

/// How long after creation the probe stops waiting for an answer.
pub(crate) const GIVE_UP_AFTER: Duration = Duration::from_millis(1500);

/// Where a probe is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Waiting; `next` indexes the next entry of [`RETRANSMIT_SCHEDULE`] to send.
    Pending { next: usize },
    /// A matching response arrived.
    Answered(SocketAddr),
    /// [`GIVE_UP_AFTER`] elapsed with no matching response.
    TimedOut,
}

/// One STUN binding probe to one server, driven by caller-supplied monotonic time.
#[derive(Debug, Clone)]
pub struct StunProbe {
    txid: TransactionId,
    started: Duration,
    state: State,
    request: [u8; REQUEST_LEN],
}

impl StunProbe {
    /// Starts a probe at monotonic time `now`. The first transmission is due immediately.
    pub fn new(txid: TransactionId, now: Duration) -> Self {
        let mut request = [0_u8; REQUEST_LEN];
        encode_binding_request(txid, &mut request);
        Self {
            txid,
            started: now,
            state: State::Pending { next: 0 },
            request,
        }
    }

    /// The transaction ID this probe matches responses against.
    pub const fn transaction_id(&self) -> TransactionId {
        self.txid
    }

    /// Returns the request to send if a transmission is due at `now`.
    ///
    /// If several are overdue (the caller polled late) only one is sent. Once
    /// 1.5 s have elapsed the probe times out and this returns `None`.
    pub fn poll_transmit(&mut self, now: Duration) -> Option<&[u8]> {
        let State::Pending { next } = self.state else {
            return None;
        };
        if now >= self.at(GIVE_UP_AFTER) {
            self.state = State::TimedOut;
            return None;
        }
        let due = RETRANSMIT_SCHEDULE
            .get(next)
            .is_some_and(|&offset| now >= self.at(offset));
        if !due {
            return None;
        }
        let skipped = RETRANSMIT_SCHEDULE
            .iter()
            .skip(next)
            .take_while(|&&offset| now >= self.at(offset))
            .count();
        self.state = State::Pending {
            next: next.saturating_add(skipped),
        };
        Some(&self.request)
    }

    /// When the caller should next call [`poll_transmit`](Self::poll_transmit):
    /// the next retransmission, or the give-up deadline. `None` once finished.
    pub fn poll_timeout(&self) -> Option<Duration> {
        let State::Pending { next } = self.state else {
            return None;
        };
        let offset = RETRANSMIT_SCHEDULE
            .get(next)
            .copied()
            .unwrap_or(GIVE_UP_AFTER);
        Some(self.at(offset))
    }

    /// Offers a received datagram; returns the reflexive address on the first matching response.
    pub fn handle_response(&mut self, packet: &[u8]) -> Option<SocketAddr> {
        let State::Pending { .. } = self.state else {
            return None;
        };
        let (txid, address) = parse_binding_response(packet).ok()?;
        if txid != self.txid {
            return None;
        }
        self.state = State::Answered(address);
        Some(address)
    }

    /// The reflexive address, once a matching response has arrived.
    pub const fn address(&self) -> Option<SocketAddr> {
        match self.state {
            State::Answered(address) => Some(address),
            State::Pending { .. } | State::TimedOut => None,
        }
    }

    /// Whether the probe gave up without an answer.
    pub const fn is_timed_out(&self) -> bool {
        matches!(self.state, State::TimedOut)
    }

    fn at(&self, offset: Duration) -> Duration {
        self.started.saturating_add(offset)
    }
}
