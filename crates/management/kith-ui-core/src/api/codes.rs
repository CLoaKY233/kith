//! Short codes people compare by eye: a joining device's check code and key fingerprints.
//!
//! Both come from the device's public key alone, so the requester and every admin compute
//! the same code without exchanging anything but the request itself.

use kith_crypto::{Context, Hash};
use kith_types::NodeId;

/// Binds the check code hash to its one purpose.
const CHECK_CODE: Context = Context::new("kith 2026-11 join check code v1");
/// Crockford base32: no I, L, O or U, so a code read aloud is not misheard.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Characters in a check code: 30 bits, enough that a forged key matching it is out of
/// reach while an admin is looking.
const CHECK_LEN: usize = 6;

/// The check code of a device asking to join: six Crockford base32 characters from a
/// hash of its public key, shown on the device and to the admin as `K7Q·4XM`.
pub(crate) fn check_code(key: &NodeId) -> String {
    let digest = Hash::of(CHECK_CODE, key.as_bytes()).to_bytes();
    let bits = digest
        .iter()
        .take(4)
        .fold(0_u32, |bits, &byte| (bits << 8) | u32::from(byte));
    (0..CHECK_LEN)
        .rev()
        .filter_map(|index| {
            let shift = u32::try_from(index)
                .ok()?
                .saturating_mul(5)
                .saturating_add(2);
            let symbol = usize::try_from((bits >> shift) & 0x1f).ok()?;
            CROCKFORD.get(symbol).map(|&byte| char::from(byte))
        })
        .collect()
}

/// A key's short fingerprint, as `k7f3·9a21`: the leading hex of the key, which is the
/// device id, after a `k` so it reads as a key and not an address.
pub(crate) fn key_fpr(key: &NodeId) -> String {
    let hex = key.to_string();
    format!(
        "k{}·{}",
        hex.get(..3).unwrap_or_default(),
        hex.get(3..7).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_readable_and_differ_per_key() {
        let one = NodeId::from_bytes([1; 32]);
        let code = check_code(&one);
        assert_eq!(code.len(), CHECK_LEN);
        assert!(code.bytes().all(|byte| CROCKFORD.contains(&byte)));
        assert_eq!(code, check_code(&one));
        assert_ne!(code, check_code(&NodeId::from_bytes([2; 32])));
        assert_eq!(key_fpr(&NodeId::from_bytes([0x7f; 32])), "k7f7·f7f7");
    }
}
