//! The fixed primitives of WireGuard (whitepaper §5.4), for `kith-wg` only.
//!
//! These take no [`crate::Context`]: WireGuard's own construction and label strings provide
//! domain separation. Transport data uses [`crate::AeadKey`] with [`crate::Nonce::from_counter`],
//! and Diffie-Hellman uses [`crate::AgreementSecret::agree`].

use core::fmt;

use blake2::digest::consts::U16;
use blake2::digest::{Digest as _, KeyInit as _, Mac as _};
use blake2::{Blake2s256, Blake2sMac};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::AeadInOut as _;
use subtle::ConstantTimeEq as _;
use zeroize::{Zeroize as _, Zeroizing};

use crate::{AeadError, Tag};

/// Length of a `HASH` output in bytes.
const HASH_LEN: usize = 32;
/// Length of a `MAC` output (MAC1, MAC2 and the cookie) in bytes.
const MAC_LEN: usize = 16;
/// Length of an XChaCha20-Poly1305 nonce in bytes.
const XNONCE_LEN: usize = 24;

/// BLAKE2s block size, which HMAC pads its key to.
const BLOCK_LEN: usize = 64;

/// A 32-byte secret produced by the KDF. Zeroizes on drop.
pub type SecretBytes = Zeroizing<[u8; HASH_LEN]>;

/// `HASH(parts[0] || parts[1] || ...)`: unkeyed BLAKE2s-256.
#[must_use]
pub fn hash(parts: &[&[u8]]) -> [u8; HASH_LEN] {
    let mut hasher = Blake2s256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// `MAC(key, parts[0] || ...)`: keyed BLAKE2s-128. `None` if `key` is empty or over 32 bytes.
#[must_use]
pub fn mac(key: &[u8], parts: &[&[u8]]) -> Option<[u8; MAC_LEN]> {
    let mut mac = Blake2sMac::<U16>::new_from_slice(key).ok()?;
    for part in parts {
        mac.update(part);
    }
    Some(mac.finalize().into_bytes().into())
}

/// `HMAC(key, parts[0] || ...)`: RFC 2104 HMAC over BLAKE2s-256.
#[must_use]
fn hmac(key: &[u8], parts: &[&[u8]]) -> SecretBytes {
    let mut block = Zeroizing::new([0_u8; BLOCK_LEN]);
    if key.len() > BLOCK_LEN {
        let digest = Zeroizing::new(hash(&[key]));
        for (slot, byte) in block.iter_mut().zip(digest.iter()) {
            *slot = *byte;
        }
    } else {
        for (slot, byte) in block.iter_mut().zip(key) {
            *slot = *byte;
        }
    }

    for byte in block.iter_mut() {
        *byte ^= 0x36;
    }
    let mut inner = Blake2s256::new();
    inner.update(block.as_slice());
    for part in parts {
        inner.update(part);
    }
    let inner_digest: SecretBytes = Zeroizing::new(inner.finalize().into());

    for byte in block.iter_mut() {
        // Undo the inner pad and apply the outer one in a single step.
        *byte ^= 0x36 ^ 0x5c;
    }
    let mut outer = Blake2s256::new();
    outer.update(block.as_slice());
    outer.update(inner_digest.as_slice());
    Zeroizing::new(outer.finalize().into())
}

/// `KDF1(key, input)`: the first HKDF output.
#[must_use]
pub fn kdf1(key: &[u8; HASH_LEN], input: &[u8]) -> SecretBytes {
    let prk = hmac(key, &[input]);
    hmac(prk.as_slice(), &[&[1]])
}

/// `KDF2(key, input)`: the first two HKDF outputs.
#[must_use]
pub fn kdf2(key: &[u8; HASH_LEN], input: &[u8]) -> (SecretBytes, SecretBytes) {
    let prk = hmac(key, &[input]);
    let first = hmac(prk.as_slice(), &[&[1]]);
    let second = hmac(prk.as_slice(), &[first.as_slice(), &[2]]);
    (first, second)
}

/// `KDF3(key, input)`: the first three HKDF outputs.
#[must_use]
pub fn kdf3(key: &[u8; HASH_LEN], input: &[u8]) -> (SecretBytes, SecretBytes, SecretBytes) {
    let prk = hmac(key, &[input]);
    let first = hmac(prk.as_slice(), &[&[1]]);
    let second = hmac(prk.as_slice(), &[first.as_slice(), &[2]]);
    let third = hmac(prk.as_slice(), &[second.as_slice(), &[3]]);
    (first, second, third)
}

/// Compares two byte strings in time that depends only on their lengths.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// An XChaCha20-Poly1305 key, used only to seal WireGuard cookie replies. Zeroizes on drop.
pub struct CookieAeadKey(XChaCha20Poly1305);

impl CookieAeadKey {
    /// Builds a key from 32 secret bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; HASH_LEN]) -> Self {
        let mut key = (*bytes).into();
        let cipher = XChaCha20Poly1305::new(&key);
        key.zeroize();
        Self(cipher)
    }

    /// Encrypts `buffer` in place and returns the tag.
    pub fn seal_in_place(
        &self,
        nonce: &[u8; XNONCE_LEN],
        associated_data: &[u8],
        buffer: &mut [u8],
    ) -> Result<Tag, AeadError> {
        let tag = self
            .0
            .encrypt_inout_detached(&(*nonce).into(), associated_data, buffer.into())
            .map_err(|_| AeadError)?;
        Ok(Tag::from_bytes(tag.into()))
    }

    /// Decrypts `buffer` in place after checking `tag`. On failure `buffer` is unspecified.
    pub fn open_in_place(
        &self,
        nonce: &[u8; XNONCE_LEN],
        associated_data: &[u8],
        buffer: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        self.0
            .decrypt_inout_detached(
                &(*nonce).into(),
                associated_data,
                buffer.into(),
                &tag.to_bytes().into(),
            )
            .map_err(|_| AeadError)
    }
}

impl fmt::Debug for CookieAeadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CookieAeadKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::{hash, hmac, kdf1, kdf2, kdf3, mac};

    fn hex(bytes: &[u8]) -> alloc::string::String {
        use core::fmt::Write as _;
        bytes.iter().fold(alloc::string::String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// Known answers. `hash("abc")` is the RFC 7693 appendix vector; the rest were computed
    /// with Python's independent `hashlib.blake2s` and `hmac` modules. They pin the exact
    /// HMAC padding, the long-key path and the HKDF chaining (`T(i) = HMAC(PRK, T(i-1) || i)`).
    #[test]
    fn primitives_match_independent_reference_vectors() {
        let key: [u8; 32] = core::array::from_fn(|i| u8::try_from(i).unwrap());
        let long_key: [u8; 100] = core::array::from_fn(|i| u8::try_from(i).unwrap());
        let message = b"The quick brown fox jumps over the lazy dog";

        assert_eq!(
            hex(&hash(&[b"a", b"bc"])),
            "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982"
        );
        assert_eq!(
            hex(&mac(&key, &[message]).unwrap()),
            "4e2219ec6d234a478e4ace91429bbce4"
        );
        assert_eq!(
            hex(hmac(&key, &[message]).as_slice()),
            "f4660dee21c6a48e2a3d695ceedc26693e37b64b5ac5227476578517ee87fe6d"
        );
        assert_eq!(
            hex(hmac(&long_key, &[message]).as_slice()),
            "62bc55ee982e83e9f80ff756c037bac835c4201c91817685b52396da1482a6e4"
        );

        let t1 = "c471865b3d2b771612913b81650c7c69ca16c5ac72ae83aaf2e47dc5337e67fc";
        let t2 = "2411eee3c28d42aa1059e1d4a1d9622de9d3af8ecc67a0a2f7775a4af463a1df";
        let t3 = "c909be26cedc47c1b8e0dad92bfb74fde4fc061980c7d398142c5e0a3c48f4e1";
        assert_eq!(hex(kdf1(&key, message).as_slice()), t1);
        let (a, b) = kdf2(&key, message);
        assert_eq!(
            (hex(a.as_slice()), hex(b.as_slice())),
            (t1.into(), t2.into())
        );
        let (a, b, c) = kdf3(&key, message);
        assert_eq!(
            (hex(a.as_slice()), hex(b.as_slice()), hex(c.as_slice())),
            (t1.into(), t2.into(), t3.into())
        );
    }
}
