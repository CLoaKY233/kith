//! Obfuscation parameters. They arrive from the network (ledger or AmneziaWG
//! config), so every constructor and deserialization validates them.

use serde::{Deserialize, Serialize};

use crate::random::{RandomSource, uniform_u16, uniform_u32};
use crate::transform::MessageType;

/// Size of a WireGuard handshake initiation message.
pub const WG_INITIATION_LEN: usize = 148;
/// Size of a WireGuard handshake response message.
pub const WG_RESPONSE_LEN: usize = 92;
/// Size of a WireGuard cookie reply message.
pub const WG_COOKIE_LEN: usize = 64;
/// Minimum size of a WireGuard transport data message (header, counter, tag).
pub const WG_TRANSPORT_MIN_LEN: usize = 32;

/// Largest datagram AmneziaWG accepts (`MaxSegmentSize` in amneziawg-go).
/// Junk and padded handshakes must be strictly smaller.
const MAX_SEGMENT_SIZE: usize = 65_535;

/// Upper bound on `Jc`. amneziawg-go only requires `Jc >= 0`, but the
/// AmneziaWG docs specify `Jc <= 128`, and an unbounded count would let a
/// hostile config amplify every handshake.
pub const MAX_JUNK_COUNT: u8 = 128;

/// Unvalidated obfuscation parameters, in AmneziaWG terms. [`Default`] is plain WireGuard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RawObfsParams {
    /// `Jc`: junk packets sent before each handshake initiation.
    pub junk_count: u8,
    /// `Jmin`: minimum junk packet size in bytes.
    pub junk_min: u16,
    /// `Jmax`: maximum junk packet size in bytes.
    pub junk_max: u16,
    /// `S1`: random bytes prepended to a handshake initiation.
    pub init_padding: u16,
    /// `S2`: random bytes prepended to a handshake response.
    pub response_padding: u16,
    /// `H1`: header word for handshake initiations.
    pub header_init: u32,
    /// `H2`: header word for handshake responses.
    pub header_response: u32,
    /// `H3`: header word for cookie replies.
    pub header_cookie: u32,
    /// `H4`: header word for transport data.
    pub header_transport: u32,
}

impl Default for RawObfsParams {
    fn default() -> Self {
        ObfsParams::identity().raw()
    }
}

/// A parameter set violates an AmneziaWG 1.x constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParamsError {
    /// `Jc` is above [`MAX_JUNK_COUNT`].
    #[error("junk count {count} exceeds {MAX_JUNK_COUNT}")]
    JunkCountTooLarge {
        /// The configured `Jc`.
        count: u8,
    },
    /// `Jmin > Jmax`.
    #[error("junk minimum {min} exceeds junk maximum {max}")]
    JunkRangeInverted {
        /// The configured `Jmin`.
        min: u16,
        /// The configured `Jmax`.
        max: u16,
    },
    /// `Jmax` is not below the maximum segment size.
    #[error("junk maximum {max} must be smaller than {MAX_SEGMENT_SIZE}")]
    JunkTooLarge {
        /// The configured `Jmax`.
        max: u16,
    },
    /// `S1 + 148` is not below the maximum segment size.
    #[error("initiation padding {padding} makes the packet too large")]
    InitPaddingTooLarge {
        /// The configured `S1`.
        padding: u16,
    },
    /// `S2 + 92` is not below the maximum segment size.
    #[error("response padding {padding} makes the packet too large")]
    ResponsePaddingTooLarge {
        /// The configured `S2`.
        padding: u16,
    },
    /// `S1 + 148 == S2 + 92`, so the receiver could not tell an initiation
    /// from a response by size.
    #[error("padded initiation and response would both be {size} bytes")]
    PaddedSizesCollide {
        /// The shared padded size.
        size: usize,
    },
    /// Two of `H1`..`H4` are equal.
    #[error("header values H1..H4 must be distinct")]
    DuplicateHeaders,
}

/// A validated AmneziaWG 1.x obfuscation parameter set.
///
/// Invariants, matching amneziawg-go's `handlePostConfig` checks:
///
/// - `Jc <= 128` ([`MAX_JUNK_COUNT`]; the AmneziaWG documented range).
/// - `Jmin <= Jmax < 65535`.
/// - `S1 + 148 < 65535` and `S2 + 92 < 65535`.
/// - `S1 + 148 != S2 + 92` (equivalently `S1 + 56 != S2`), so the two
///   handshake messages stay distinguishable by size.
/// - `H1`, `H2`, `H3`, `H4` are pairwise distinct.
///
/// `Jmin` and `Jmax` are validated even when `Jc == 0`, like upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawObfsParams", into = "RawObfsParams")]
pub struct ObfsParams {
    raw: RawObfsParams,
}

impl ObfsParams {
    /// Plain WireGuard: no junk, no padding, standard headers `1`..`4`.
    pub const fn identity() -> Self {
        Self {
            raw: RawObfsParams {
                junk_count: 0,
                junk_min: 0,
                junk_max: 0,
                init_padding: 0,
                response_padding: 0,
                header_init: 1,
                header_response: 2,
                header_cookie: 3,
                header_transport: 4,
            },
        }
    }

    /// Validates `raw` against every AmneziaWG 1.x constraint.
    pub fn new(raw: RawObfsParams) -> Result<Self, ParamsError> {
        if raw.junk_count > MAX_JUNK_COUNT {
            return Err(ParamsError::JunkCountTooLarge {
                count: raw.junk_count,
            });
        }
        if raw.junk_min > raw.junk_max {
            return Err(ParamsError::JunkRangeInverted {
                min: raw.junk_min,
                max: raw.junk_max,
            });
        }
        if usize::from(raw.junk_max) >= MAX_SEGMENT_SIZE {
            return Err(ParamsError::JunkTooLarge { max: raw.junk_max });
        }
        let init_size = usize::from(raw.init_padding).saturating_add(WG_INITIATION_LEN);
        if init_size >= MAX_SEGMENT_SIZE {
            return Err(ParamsError::InitPaddingTooLarge {
                padding: raw.init_padding,
            });
        }
        let response_size = usize::from(raw.response_padding).saturating_add(WG_RESPONSE_LEN);
        if response_size >= MAX_SEGMENT_SIZE {
            return Err(ParamsError::ResponsePaddingTooLarge {
                padding: raw.response_padding,
            });
        }
        if init_size == response_size {
            return Err(ParamsError::PaddedSizesCollide { size: init_size });
        }
        let headers = [
            raw.header_init,
            raw.header_response,
            raw.header_cookie,
            raw.header_transport,
        ];
        let duplicate = headers.iter().enumerate().any(|(index, value)| {
            headers
                .iter()
                .skip(index.saturating_add(1))
                .any(|other| other == value)
        });
        if duplicate {
            return Err(ParamsError::DuplicateHeaders);
        }
        Ok(Self { raw })
    }

    /// Generates a random valid parameter set within the AmneziaWG recommended ranges.
    ///
    /// Headers are drawn from `5..=u32::MAX` so none collides with a standard WireGuard type.
    pub fn random(rng: &mut impl RandomSource) -> Self {
        let junk_count = u8::try_from(uniform_u32(rng, 4, 12)).unwrap_or(4);
        let junk_min = uniform_u16(rng, 40, 100);
        let junk_max = uniform_u16(rng, junk_min, 1000);
        let init_padding = uniform_u16(rng, 15, 150);
        let mut response_padding = uniform_u16(rng, 15, 150);
        // S1 + 56 == S2 would make the padded sizes equal.
        if init_padding.checked_add(56) == Some(response_padding) {
            response_padding = response_padding.saturating_add(1);
        }
        let mut headers = [0_u32; 4];
        for index in 0..headers.len() {
            let mut value = uniform_u32(rng, 5, u32::MAX);
            // Resolve collisions deterministically: at most three steps, since
            // at most three earlier values can collide.
            while headers.iter().take(index).any(|earlier| *earlier == value) {
                value = value.checked_add(1).unwrap_or(5);
            }
            if let Some(slot) = headers.get_mut(index) {
                *slot = value;
            }
        }
        let [
            header_init,
            header_response,
            header_cookie,
            header_transport,
        ] = headers;
        let raw = RawObfsParams {
            junk_count,
            junk_min,
            junk_max,
            init_padding,
            response_padding,
            header_init,
            header_response,
            header_cookie,
            header_transport,
        };
        // Every draw above respects the constraints, so this cannot fail;
        // fall back to identity rather than panic.
        Self::new(raw).unwrap_or_default()
    }

    /// Whether these parameters are plain WireGuard.
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }

    /// The parameters as plain data.
    pub const fn raw(&self) -> RawObfsParams {
        self.raw
    }

    /// `Jc`: junk packets to send before each handshake initiation.
    pub const fn junk_count(&self) -> u8 {
        self.raw.junk_count
    }

    /// `Jmin`: minimum junk packet size.
    pub const fn junk_min(&self) -> u16 {
        self.raw.junk_min
    }

    /// `Jmax`: maximum junk packet size, which is also the buffer size
    /// [`ObfsParams::junk_packet`] needs.
    pub const fn junk_max(&self) -> u16 {
        self.raw.junk_max
    }

    /// Random bytes prepended to messages of `kind` (`S1`, `S2`, or zero).
    pub fn padding_len(&self, kind: MessageType) -> usize {
        match kind {
            MessageType::Initiation => usize::from(self.raw.init_padding),
            MessageType::Response => usize::from(self.raw.response_padding),
            MessageType::CookieReply | MessageType::Transport => 0,
        }
    }

    /// The header word used on the wire for messages of `kind` (`H1`..`H4`).
    pub const fn header(&self, kind: MessageType) -> u32 {
        match kind {
            MessageType::Initiation => self.raw.header_init,
            MessageType::Response => self.raw.header_response,
            MessageType::CookieReply => self.raw.header_cookie,
            MessageType::Transport => self.raw.header_transport,
        }
    }
}

impl Default for ObfsParams {
    fn default() -> Self {
        Self::identity()
    }
}

impl TryFrom<RawObfsParams> for ObfsParams {
    type Error = ParamsError;

    fn try_from(raw: RawObfsParams) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}

impl From<ObfsParams> for RawObfsParams {
    fn from(params: ObfsParams) -> Self {
        params.raw
    }
}
