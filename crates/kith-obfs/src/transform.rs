//! The outbound and inbound packet transforms.
//!
//! Outbound works on WireGuard messages that are already sealed. Inbound
//! classifies datagrams by size and header word, exactly as amneziawg-go
//! does, so the two interoperate. Neither direction allocates or panics.

use crate::params::{
    ObfsParams, WG_COOKIE_LEN, WG_INITIATION_LEN, WG_RESPONSE_LEN, WG_TRANSPORT_MIN_LEN,
};
use crate::random::{RandomSource, uniform_u16};

/// Size of the WireGuard header word that the transform rewrites.
const HEADER_LEN: usize = 4;

/// The four WireGuard message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// Handshake initiation (type 1, 148 bytes).
    Initiation,
    /// Handshake response (type 2, 92 bytes).
    Response,
    /// Cookie reply (type 3, 64 bytes).
    CookieReply,
    /// Transport data (type 4, at least 32 bytes).
    Transport,
}

impl MessageType {
    /// The standard WireGuard header word for this type.
    pub const fn wireguard_header(self) -> u32 {
        match self {
            Self::Initiation => 1,
            Self::Response => 2,
            Self::CookieReply => 3,
            Self::Transport => 4,
        }
    }

    /// Classifies a plain WireGuard message by its header word and size.
    ///
    /// Returns `None` unless the header is a standard type (with zero reserved
    /// bytes) and the size is valid for it.
    pub fn of_wireguard(message: &[u8]) -> Option<Self> {
        let kind = match read_header(message, 0)? {
            1 => Self::Initiation,
            2 => Self::Response,
            3 => Self::CookieReply,
            4 => Self::Transport,
            _ => return None,
        };
        kind.size_matches(message.len()).then_some(kind)
    }

    /// Whether `len` is a valid unpadded size for this type.
    const fn size_matches(self, len: usize) -> bool {
        match self {
            Self::Initiation => len == WG_INITIATION_LEN,
            Self::Response => len == WG_RESPONSE_LEN,
            Self::CookieReply => len == WG_COOKIE_LEN,
            Self::Transport => len >= WG_TRANSPORT_MIN_LEN,
        }
    }
}

/// An outbound transform could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The input is not a well-formed WireGuard message.
    #[error("input is not a WireGuard message")]
    NotWireguard,
    /// The output buffer is too small.
    #[error("output buffer too small: {needed} bytes needed")]
    BufferTooSmall {
        /// Bytes the output needs.
        needed: usize,
    },
    /// The message needs padding, so it cannot be transformed in place.
    #[error("message needs padding; use `encode`")]
    NeedsPadding,
}

/// The result of classifying an inbound datagram.
#[derive(Debug, PartialEq, Eq)]
pub enum Inbound<'a> {
    /// A WireGuard message. Padding is stripped and the standard header is
    /// restored, so `message` can go straight to the WireGuard state machine.
    Wireguard {
        /// The message type.
        kind: MessageType,
        /// The restored message, borrowed from the datagram.
        message: &'a mut [u8],
    },
    /// No message matched, and the size fits the junk range: drop it.
    Junk,
    /// No message matched, and it is not junk-sized: drop it (and perhaps
    /// count it).
    Unrecognized,
}

impl ObfsParams {
    /// Obfuscates a sealed WireGuard `message` into `out`.
    ///
    /// Writes the padding (random bytes) and then the message with its header
    /// remapped. Returns the number of bytes written. Callers send the
    /// [`junk_count`](Self::junk_count) junk packets from
    /// [`junk_packet`](Self::junk_packet) before each initiation.
    ///
    /// For transport data prefer [`encode_in_place`](Self::encode_in_place),
    /// which does not copy.
    pub fn encode(
        &self,
        message: &[u8],
        out: &mut [u8],
        rng: &mut impl RandomSource,
    ) -> Result<usize, EncodeError> {
        let kind = MessageType::of_wireguard(message).ok_or(EncodeError::NotWireguard)?;
        let padding = self.padding_len(kind);
        let needed = padding
            .checked_add(message.len())
            .ok_or(EncodeError::BufferTooSmall { needed: usize::MAX })?;
        let out = out
            .get_mut(..needed)
            .ok_or(EncodeError::BufferTooSmall { needed })?;
        let (pad, body) = out
            .split_at_mut_checked(padding)
            .ok_or(EncodeError::BufferTooSmall { needed })?;
        rng.fill(pad);
        body.copy_from_slice(message);
        write_header(body, self.header(kind));
        Ok(needed)
    }

    /// Obfuscates a message that needs no padding by rewriting its header in
    /// place. This is the transport-data hot path: a 4-byte write.
    ///
    /// Fails with [`EncodeError::NeedsPadding`] for handshake messages when
    /// `S1`/`S2` are non-zero.
    pub fn encode_in_place(&self, message: &mut [u8]) -> Result<MessageType, EncodeError> {
        let kind = MessageType::of_wireguard(message).ok_or(EncodeError::NotWireguard)?;
        if self.padding_len(kind) != 0 {
            return Err(EncodeError::NeedsPadding);
        }
        write_header(message, self.header(kind));
        Ok(kind)
    }

    /// Writes one junk packet of random length in `[Jmin, Jmax]` into `out`
    /// and returns its length. `out` must hold at least `Jmax` bytes.
    pub fn junk_packet(
        &self,
        out: &mut [u8],
        rng: &mut impl RandomSource,
    ) -> Result<usize, EncodeError> {
        let max = usize::from(self.junk_max());
        if out.len() < max {
            return Err(EncodeError::BufferTooSmall { needed: max });
        }
        let len = usize::from(uniform_u16(rng, self.junk_min(), self.junk_max()));
        let packet = out
            .get_mut(..len)
            .ok_or(EncodeError::BufferTooSmall { needed: len })?;
        rng.fill(packet);
        Ok(len)
    }

    /// Classifies an inbound datagram and, if it is a WireGuard message,
    /// strips its padding and restores the standard header in place.
    ///
    /// Rules are tried in amneziawg-go's order:
    ///
    /// 1. size `S1 + 148` with `H1` at offset `S1`: initiation;
    /// 2. size `S2 + 92` with `H2` at offset `S2`: response;
    /// 3. size 64 with `H3` at offset 0: cookie reply;
    /// 4. size at least 32 with `H4` at offset 0: transport data;
    /// 5. otherwise junk if `Jc > 0` and the size is in `[Jmin, Jmax]`,
    ///    else unrecognized.
    ///
    /// Never panics, for any input.
    pub fn decode<'a>(&self, datagram: &'a mut [u8]) -> Inbound<'a> {
        let len = datagram.len();
        let matched = [
            MessageType::Initiation,
            MessageType::Response,
            MessageType::CookieReply,
            MessageType::Transport,
        ]
        .into_iter()
        .find(|&kind| {
            let padding = self.padding_len(kind);
            len.checked_sub(padding)
                .is_some_and(|body| kind.size_matches(body))
                && read_header(datagram, padding) == Some(self.header(kind))
        });
        if let Some(kind) = matched
            && let Some(message) = datagram.get_mut(self.padding_len(kind)..)
        {
            write_header(message, kind.wireguard_header());
            return Inbound::Wireguard { kind, message };
        }
        let junk_range = usize::from(self.junk_min())..=usize::from(self.junk_max());
        if self.junk_count() > 0 && junk_range.contains(&len) {
            Inbound::Junk
        } else {
            Inbound::Unrecognized
        }
    }
}

/// Reads the little-endian header word at `offset`, if the bytes exist.
fn read_header(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(HEADER_LEN)?;
    let word: [u8; HEADER_LEN] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(word))
}

/// Overwrites the leading header word. No-op if `message` is too short,
/// which callers rule out by validating sizes first.
fn write_header(message: &mut [u8], value: u32) {
    if let Some(header) = message.get_mut(..HEADER_LEN) {
        header.copy_from_slice(&value.to_le_bytes());
    }
}
