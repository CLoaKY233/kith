//! Every domain-separation context owned by this crate.
//!
//! Convention: `"kith <year-month> <purpose> v<n>"`. Changing any is a protocol break.

use kith_crypto::Context;

/// Checksum over the seed in a secret-key file, to detect corruption.
pub const SECRET_KEY_CHECKSUM: Context = Context::new("kith 2026-10 secret key checksum v1");

/// Truncated checksum appended to an invite payload, to catch typos in links.
pub const INVITE_CHECKSUM: Context = Context::new("kith 2026-10 invite checksum v1");

/// Every context above, for uniqueness tests and audits.
pub const ALL: &[Context] = &[SECRET_KEY_CHECKSUM, INVITE_CHECKSUM];

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn contexts_are_unique_and_follow_the_convention() {
        for (index, context) in ALL.iter().enumerate() {
            let text = context.as_str();
            assert!(text.starts_with("kith 2026-10 "), "{text}");
            assert!(text.ends_with(" v1"), "{text}");
            for other in ALL.iter().skip(index + 1) {
                assert_ne!(text, other.as_str());
            }
        }
    }
}
