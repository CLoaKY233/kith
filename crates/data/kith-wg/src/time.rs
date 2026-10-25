//! Time as an input, and the protocol's timer constants (whitepaper section 6.1).

use core::time::Duration;

/// Start a new handshake once a session has sent this many messages.
pub(crate) const REKEY_AFTER_MESSAGES: u64 = 1 << 60;
/// Never send or accept a message with a counter at or above this.
pub(crate) const REJECT_AFTER_MESSAGES: u64 = u64::MAX - (1 << 13);
/// The initiator of a session starts a new handshake once the session is this old.
pub(crate) const REKEY_AFTER_TIME: Duration = Duration::from_secs(120);
/// A session older than this is never used again.
pub(crate) const REJECT_AFTER_TIME: Duration = Duration::from_secs(180);
/// Give up on a handshake after retrying it for this long.
pub(crate) const REKEY_ATTEMPT_TIME: Duration = Duration::from_secs(90);
/// Retransmit a handshake initiation if no response arrived within this (plus jitter).
pub(crate) const REKEY_TIMEOUT: Duration = Duration::from_secs(5);
/// Send a keepalive if we received data but sent nothing for this long.
pub(crate) const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on the random jitter added to each handshake retransmission.
pub(crate) const REKEY_TIMEOUT_JITTER_MS: u32 = 334;
/// A cookie received from a peer is used for MAC2 for this long.
pub(crate) const COOKIE_LIFETIME: Duration = Duration::from_secs(120);
/// The responder's cookie secret is replaced this often.
pub(crate) const COOKIE_SECRET_LIFETIME: Duration = Duration::from_secs(120);
/// All key material is erased if no new session was made for this long.
pub(crate) const ZERO_KEYS_AFTER: Duration = Duration::from_mins(9);

/// A TAI64N timestamp: 8 bytes of big-endian TAI64 seconds, 4 of big-endian nanoseconds.
///
/// The byte order makes lexicographic comparison equal to chronological comparison, which
/// is how the responder detects replayed initiations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tai64N([u8; 12]);

impl Tai64N {
    /// The TAI64 label of the Unix epoch, as WireGuard implementations encode it.
    const UNIX_EPOCH_LABEL: u64 = (1 << 62) + 10;

    /// Wraps raw TAI64N bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 12]) -> Self {
        Self(bytes)
    }

    /// Encodes a time given as a duration since the Unix epoch.
    #[must_use]
    pub fn from_unix(since_epoch: Duration) -> Self {
        let mut bytes = [0_u8; 12];
        let seconds = since_epoch
            .as_secs()
            .saturating_add(Self::UNIX_EPOCH_LABEL)
            .to_be_bytes();
        let nanos = since_epoch.subsec_nanos().to_be_bytes();
        for (slot, byte) in bytes.iter_mut().zip(seconds.iter().chain(nanos.iter())) {
            *slot = *byte;
        }
        Self(bytes)
    }

    /// The raw bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 12] {
        self.0
    }
}

/// The current time, as the caller sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Now {
    /// Monotonic time since an arbitrary origin; drives every timer. Must never go back.
    pub monotonic: Duration,
    /// Wall-clock time, placed in handshake initiations. Must increase across restarts.
    pub wall: Tai64N,
}
