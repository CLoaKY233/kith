//! Hand-built Ed25519 DER: `SubjectPublicKeyInfo` (RFC 7250/8410) and PKCS#8.
//!
//! No certificate machinery. The RPK path presents and verifies these fixed-shape blobs
//! directly, so the prefixes are constants rather than a general DER encoder.

use kith_crypto::VerifyingKey;
use kith_types::NodeId;
use zeroize::Zeroizing;

/// Raw key byte count for Ed25519.
const KEY_LEN: usize = 32;

/// SPKI prefix for an Ed25519 public key (RFC 8410 §4): the `SEQUENCE` of `AlgorithmIdentifier` and `BIT STRING`.
const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// PKCS#8 v1 prefix for an Ed25519 seed (RFC 8410 §7): the `OneAsymmetricKey` header.
const PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Length of a complete Ed25519 SPKI blob.
pub(crate) const SPKI_LEN: usize = SPKI_PREFIX.len() + KEY_LEN;

/// Builds the `SubjectPublicKeyInfo` for an Ed25519 public key.
pub(crate) fn spki(public_key: &[u8; KEY_LEN]) -> [u8; SPKI_LEN] {
    let mut out = [0_u8; SPKI_LEN];
    let (head, tail) = out.split_at_mut(SPKI_PREFIX.len());
    head.copy_from_slice(&SPKI_PREFIX);
    tail.copy_from_slice(public_key);
    out
}

/// Builds the PKCS#8 encoding of an Ed25519 seed. Zeroizes on drop.
pub(crate) fn pkcs8(seed: &[u8; KEY_LEN]) -> Zeroizing<[u8; PKCS8_PREFIX.len() + KEY_LEN]> {
    let mut out = Zeroizing::new([0_u8; PKCS8_PREFIX.len() + KEY_LEN]);
    let (head, tail) = out.split_at_mut(PKCS8_PREFIX.len());
    head.copy_from_slice(&PKCS8_PREFIX);
    tail.copy_from_slice(seed);
    out
}

/// Parses a peer's SPKI blob into its identity, rejecting the wrong shape or a weak key.
pub(crate) fn node_id_from_spki(bytes: &[u8]) -> Option<NodeId> {
    let prefix = bytes.get(..SPKI_PREFIX.len());
    let key = bytes.get(SPKI_PREFIX.len()..);
    match (prefix, key) {
        (Some(prefix), Some(key)) if prefix == SPKI_PREFIX && key.len() == KEY_LEN => {
            let mut raw = [0_u8; KEY_LEN];
            raw.copy_from_slice(key);
            VerifyingKey::from_bytes(&raw).ok().map(|k| k.node_id())
        }
        _ => None,
    }
}
