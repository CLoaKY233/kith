//! Every domain-separation context owned by this crate.
//!
//! Convention: `"kith <year-month> <purpose> v<n>"`. Changing any is a protocol break.

use kith_crypto::Context;

/// Checksum over the seed in a secret-key file, to detect corruption.
pub(crate) const SECRET_KEY_CHECKSUM: Context = Context::new("kith 2026-10 secret key checksum v1");

/// Truncated checksum appended to an invite payload, to catch typos in links.
pub(crate) const INVITE_CHECKSUM: Context = Context::new("kith 2026-10 invite checksum v1");

/// Truncated checksum appended to a request code payload.
pub(crate) const REQUEST_CODE_CHECKSUM: Context =
    Context::new("kith 2026-10 request code checksum v1");

/// Derives a network's recovery key from the secret in its recovery code.
pub(crate) const RECOVERY_KEY: Context = Context::new("kith 2026-11 recovery key v1");

/// Truncated checksum appended to a recovery code, to catch a wrong word.
pub(crate) const RECOVERY_CHECKSUM: Context = Context::new("kith 2026-11 recovery checksum v1");

/// Every context above, for uniqueness tests and audits.
#[cfg(test)]
pub(crate) const ALL: &[Context] = &[
    SECRET_KEY_CHECKSUM,
    INVITE_CHECKSUM,
    REQUEST_CODE_CHECKSUM,
    RECOVERY_KEY,
    RECOVERY_CHECKSUM,
];

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn contexts_are_unique_and_follow_the_convention() {
        for (index, context) in ALL.iter().enumerate() {
            let text = context.as_str();
            assert!(text.starts_with("kith 2026-"), "{text}");
            assert!(text.ends_with(" v1"), "{text}");
            for other in ALL.iter().skip(index + 1) {
                assert_ne!(text, other.as_str());
            }
        }
    }
}
