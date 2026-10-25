//! Sealed disco packets.
//!
//! ```text
//! header "kt" | 0x50 | 0x01 (4) | sender disco public key (32) | nonce (24) | ciphertext | tag (16)
//! ```
//!
//! The key is `derive_key("kith 2026-10 disco seal v1", X25519(sender, receiver) || nonce)`.
//! Because every packet carries a fresh random 24-byte nonce, every packet is encrypted under its
//! own key, so ChaCha20-Poly1305 runs with an all-zero nonce: a (key, nonce) pair can only repeat
//! if a 192-bit random value repeats. This gives XChaCha-like nonce safety with the plain
//! ChaCha20-Poly1305 that `kith-crypto` exposes.
//!
//! Everything before the ciphertext (header, sender key and nonce) is associated data, so no
//! byte of the packet can be changed without failing authentication.
//!
//! The four header bytes also let the UDP demultiplexer tell disco from other traffic (see
//! [`is_disco`]).

use alloc::vec::Vec;

use kith_crypto::{
    AeadKey, AgreementPublic, AgreementSecret, Context, Nonce, RandomError, TAG_LEN, derive_key,
};
use kith_wire::{MAGIC, envelope, registry::DISCO_MESSAGE};

use crate::message::{MAX_PLAINTEXT_LEN, Message, ParseError};

/// The key-derivation context for sealing disco packets.
pub const SEAL_CONTEXT: Context = Context::new("kith 2026-10 disco seal v1");

/// The disco packet format version carried in the envelope header.
pub const VERSION: u8 = 1;

/// The fixed header: kith magic, [`DISCO_MESSAGE`], version 1 (a one-byte varint).
const HEADER: [u8; 4] = [MAGIC[0], MAGIC[1], DISCO_MESSAGE.0, VERSION];
const KEY_LEN: usize = 32;
/// The length of the random per-packet nonce.
pub const NONCE_LEN: usize = 24;
const PREFIX_LEN: usize = HEADER.len() + KEY_LEN + NONCE_LEN;

/// The smallest possible sealed packet: an empty plaintext. Real messages are longer.
pub const MIN_PACKET_LEN: usize = PREFIX_LEN + TAG_LEN;
/// The largest sealed packet a valid message can produce.
pub const MAX_PACKET_LEN: usize = MIN_PACKET_LEN + MAX_PLAINTEXT_LEN;

/// A source of cryptographically secure random bytes, supplied by the driver.
pub trait RandomSource {
    /// Fills `buffer` with random bytes.
    fn fill(&mut self, buffer: &mut [u8]) -> Result<(), RandomError>;
}

/// Why a packet could not be sealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SealError {
    /// The random source failed.
    #[error("random source failed")]
    Random,
    /// Key agreement failed: the peer key is low-order.
    #[error("invalid peer disco key")]
    BadPeerKey,
    /// Encryption failed.
    #[error("encryption failed")]
    Encrypt,
}

/// Why a packet could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OpenError {
    /// Not a version 1 disco packet, or too short or too long to be one.
    #[error("not a disco packet")]
    NotDisco,
    /// Key agreement failed: the sender key is low-order.
    #[error("invalid sender disco key")]
    BadSenderKey,
    /// Authentication failed: tampered, truncated, or sealed for someone else.
    #[error("authentication failed")]
    Authentication,
    /// The packet authenticated but its plaintext is not a valid message.
    #[error("malformed message: {0}")]
    Malformed(#[from] ParseError),
}

/// Whether `packet` looks like a disco packet, judged by its first bytes only.
///
/// This cannot collide with the other UDP traffic on the socket: WireGuard messages start with a
/// type byte of 1 to 4 followed by three zero bytes, and STUN messages start with two zero bits,
/// whereas `'k'` is `0x6b` (`0b01...`).
pub fn is_disco(packet: &[u8]) -> bool {
    packet.starts_with(&HEADER)
}

/// Seals `message` from `local` to `peer`.
pub fn seal(
    message: &Message,
    local: &AgreementSecret,
    peer: &AgreementPublic,
    random: &mut impl RandomSource,
) -> Result<Vec<u8>, SealError> {
    let mut nonce = [0_u8; NONCE_LEN];
    random.fill(&mut nonce).map_err(|_| SealError::Random)?;
    let key = packet_key(local, peer, &nonce).ok_or(SealError::BadPeerKey)?;

    let mut packet = Vec::with_capacity(MAX_PACKET_LEN);
    packet.extend_from_slice(&HEADER);
    packet.extend_from_slice(&local.public_key().to_bytes());
    packet.extend_from_slice(&nonce);
    let mut plaintext = Vec::with_capacity(MAX_PLAINTEXT_LEN);
    message.encode(&mut plaintext);
    let sealed = key
        .seal(Nonce::from_bytes([0; 12]), &packet, &plaintext)
        .map_err(|_| SealError::Encrypt)?;
    packet.extend_from_slice(&sealed);
    Ok(packet)
}

/// Opens a packet sealed to `local`, returning the sender's disco key and the message.
///
/// The caller must check that the returned key belongs to a known peer.
pub fn open(
    packet: &[u8],
    local: &AgreementSecret,
) -> Result<(AgreementPublic, Message), OpenError> {
    if !(MIN_PACKET_LEN..=MAX_PACKET_LEN).contains(&packet.len()) {
        return Err(OpenError::NotDisco);
    }
    let (header, body) = envelope::decode_header(packet).map_err(|_| OpenError::NotDisco)?;
    if header.message_type != DISCO_MESSAGE || header.version != u64::from(VERSION) {
        return Err(OpenError::NotDisco);
    }
    let (sender, rest) = body
        .split_first_chunk::<KEY_LEN>()
        .ok_or(OpenError::NotDisco)?;
    let (nonce, sealed) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(OpenError::NotDisco)?;
    let associated = packet.get(..PREFIX_LEN).ok_or(OpenError::NotDisco)?;
    let sender = AgreementPublic::from_bytes(*sender);
    let key = packet_key(local, &sender, nonce).ok_or(OpenError::BadSenderKey)?;
    let plaintext = key
        .open(Nonce::from_bytes([0; 12]), associated, sealed)
        .map_err(|_| OpenError::Authentication)?;
    Ok((sender, Message::parse(&plaintext)?))
}

fn packet_key(
    local: &AgreementSecret,
    peer: &AgreementPublic,
    nonce: &[u8; NONCE_LEN],
) -> Option<AeadKey> {
    let shared = local.agree(peer).ok()?;
    let mut material = zeroed_material();
    let (secret_part, nonce_part) = material.split_at_mut(KEY_LEN);
    secret_part.copy_from_slice(shared.as_bytes());
    nonce_part.copy_from_slice(nonce);
    let key = derive_key(SEAL_CONTEXT, &material);
    material.fill(0);
    Some(AeadKey::from_bytes(&key))
}

const fn zeroed_material() -> [u8; KEY_LEN + NONCE_LEN] {
    [0; KEY_LEN + NONCE_LEN]
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use kith_crypto::{AgreementSecret, RandomError};
    use kith_types::NodeId;
    use proptest::prelude::*;

    use super::{OpenError, RandomSource, is_disco, open, seal};
    use crate::message::{Message, Ping};

    struct Fixed(u8);

    impl RandomSource for Fixed {
        fn fill(&mut self, buffer: &mut [u8]) -> Result<(), RandomError> {
            buffer.fill(self.0);
            Ok(())
        }
    }

    fn keys() -> (AgreementSecret, AgreementSecret) {
        (
            AgreementSecret::from_bytes([1; 32]),
            AgreementSecret::from_bytes([2; 32]),
        )
    }

    fn ping() -> Message {
        Message::Ping(Ping {
            tx_id: [7; 12],
            sender: NodeId::from_bytes([9; 32]),
        })
    }

    fn sealed() -> Vec<u8> {
        let (alice, bob) = keys();
        seal(&ping(), &alice, &bob.public_key(), &mut Fixed(0x55)).unwrap()
    }

    #[test]
    fn opens_what_was_sealed_and_reports_the_sender() {
        let (alice, bob) = keys();
        let packet = sealed();
        assert!(is_disco(&packet));
        assert_eq!(open(&packet, &bob), Ok((alice.public_key(), ping())));
    }

    #[test]
    fn every_flipped_bit_and_every_truncation_is_rejected() {
        let (_, bob) = keys();
        let packet = sealed();
        for index in 0..packet.len() {
            let mut tampered = packet.clone();
            tampered[index] ^= 0x01;
            assert!(open(&tampered, &bob).is_err(), "byte {index}");
            assert!(open(&packet[..index], &bob).is_err(), "length {index}");
        }
    }

    #[test]
    fn a_third_party_cannot_open() {
        let eve = AgreementSecret::from_bytes([3; 32]);
        assert_eq!(open(&sealed(), &eve), Err(OpenError::Authentication));
    }

    /// Changing these bytes is a protocol break.
    #[test]
    fn golden_bytes() {
        assert_eq!(sealed(), GOLDEN);
    }

    const GOLDEN: [u8; 121] = [
        0x6b, 0x74, 0x50, 0x01, 0xa4, 0xe0, 0x92, 0x92, 0xb6, 0x51, 0xc2, 0x78, 0xb9, 0x77, 0x2c,
        0x56, 0x9f, 0x5f, 0xa9, 0xbb, 0x13, 0xd9, 0x06, 0xb4, 0x6a, 0xb6, 0x8c, 0x9d, 0xf9, 0xdc,
        0x2b, 0x44, 0x09, 0xf8, 0xa2, 0x09, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55,
        0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55,
        0x68, 0x92, 0xa9, 0x20, 0xd7, 0x49, 0xd5, 0x4d, 0x36, 0xd0, 0x17, 0xdd, 0xcc, 0x20, 0x1a,
        0xa7, 0xa3, 0xb0, 0x22, 0x26, 0xf6, 0xea, 0xdf, 0xcd, 0x6e, 0x3d, 0x20, 0x75, 0x6c, 0x81,
        0xa4, 0x55, 0x25, 0x22, 0xfe, 0x60, 0x98, 0x9d, 0x91, 0xd6, 0x0c, 0x10, 0x5d, 0xe4, 0x5b,
        0x16, 0x06, 0x1c, 0x49, 0x4b, 0x34, 0xf2, 0x22, 0x4e, 0xe4, 0xaf, 0xcb, 0x66, 0x30, 0xb5,
        0x74,
    ];

    proptest! {
        #[test]
        fn open_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..800)) {
            let (_, bob) = keys();
            // Half the inputs carry a valid header, so they reach the cryptography.
            let mut packet = if bytes.first().is_some_and(|b| b & 1 == 1) { super::HEADER.to_vec() } else { Vec::new() };
            packet.extend_from_slice(&bytes);
            prop_assert!(open(&packet, &bob).is_err());
        }
    }
}
