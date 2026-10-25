//! Size limits for everything that arrives from the network.
//!
//! The whole-op byte limit is checked before decoding, so it bounds every allocation.

/// Largest encoded [`UnsignedOp`](crate::UnsignedOp), in bytes: 64 KiB.
pub(crate) const MAX_OP_LEN: usize = 64 * 1024;

/// Most signatures one [`SignedOp`](crate::SignedOp) may carry.
pub const MAX_SIGNATURES: usize = 32;

/// Largest encoded [`SignedOp`](crate::SignedOp), in bytes: the unsigned op, every
/// signature (32-byte signer plus 64-byte signature) and some framing slack.
pub const MAX_SIGNED_OP_LEN: usize = MAX_OP_LEN + MAX_SIGNATURES * (32 + 64) + 64;

/// Most parents a non-genesis op may name.
pub const MAX_PARENTS: usize = 16;

/// Most admins a network may have at once.
pub const MAX_ADMINS: usize = 32;

/// Longest network or device name, in UTF-8 bytes.
pub const MAX_NAME_LEN: usize = 64;

/// Most tags on one device or invite.
pub const MAX_TAGS: usize = 16;

/// Longest tag, in UTF-8 bytes. Tags are also non-empty.
pub const MAX_TAG_LEN: usize = 32;

/// Longest record key, in UTF-8 bytes. Keys are also non-empty.
pub const MAX_RECORD_KEY_LEN: usize = 128;

/// Largest record value, in bytes.
pub const MAX_RECORD_VALUE_LEN: usize = 4 * 1024;

/// Most ops held while waiting for their parents to arrive.
pub const MAX_ORPHANS: usize = 1024;
