//! Why an op was rejected.

use kith_types::NodeId;
use kith_wire::WireError;

/// A structural rejection of an op; such ops are never stored. Unauthorized ops are not
/// errors: they are stored and excluded from the view.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LedgerError {
    /// The envelope or body could not be encoded or decoded.
    #[error("wire format: {0}")]
    Wire(#[from] WireError),
    /// The encoded op exceeds its size limit.
    #[error("op is {len} bytes, limit is {max}")]
    TooLarge {
        /// The encoded length.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The envelope carries a schema version this build does not understand.
    #[error("unsupported op version {0}")]
    UnsupportedVersion(u64),
    /// A genesis op names a network or parents, or has a non-zero lamport clock.
    #[error("malformed genesis op")]
    MalformedGenesis,
    /// A non-genesis op does not name its network.
    #[error("op does not name its network")]
    MissingNetwork,
    /// The op belongs to a different network.
    #[error("op belongs to a different network")]
    NetworkMismatch,
    /// [`Ledger::new`](crate::Ledger::new) was given an op that is not a genesis.
    #[error("not a genesis op")]
    NotGenesis,
    /// A second genesis op was inserted into an existing ledger.
    #[error("unexpected genesis op")]
    UnexpectedGenesis,
    /// A non-genesis op has no parents.
    #[error("op has no parents")]
    NoParents,
    /// The op names more parents than allowed.
    #[error("op has {0} parents, limit is {max}", max = crate::MAX_PARENTS)]
    TooManyParents(usize),
    /// Parents are not strictly ascending (unsorted or duplicated).
    #[error("parents are not sorted and unique")]
    ParentsUnsorted,
    /// The lamport clock is not exactly one more than the largest parent's.
    #[error("lamport is {found}, expected {expected}")]
    WrongLamport {
        /// The value the op must carry.
        expected: u64,
        /// The value it carries.
        found: u64,
    },
    /// The lamport clock would overflow.
    #[error("lamport clock overflow")]
    LamportOverflow,
    /// The op carries no signatures.
    #[error("op has no signatures")]
    NoSignatures,
    /// The op carries more signatures than allowed.
    #[error("too many signatures, limit is {max}", max = crate::MAX_SIGNATURES)]
    TooManySignatures,
    /// Signatures are not strictly ascending by signer (unsorted or duplicated).
    #[error("signatures are not sorted and unique by signer")]
    SignaturesUnsorted,
    /// A signer, admin or device identity is not a valid Ed25519 public key.
    #[error("{0:?} is not a valid public key")]
    InvalidKey(NodeId),
    /// A signature does not verify over the op's bytes.
    #[error("bad signature from {0:?}")]
    BadSignature(NodeId),
    /// The op's author did not sign it.
    #[error("the author did not sign the op")]
    AuthorNotSigner,
    /// A name is longer than allowed.
    #[error("name is {0} bytes, limit is {max}", max = crate::MAX_NAME_LEN)]
    NameTooLong(usize),
    /// Too many tags.
    #[error("{0} tags, limit is {max}", max = crate::MAX_TAGS)]
    TooManyTags(usize),
    /// A tag is empty or longer than allowed.
    #[error("tag is {0} bytes, must be 1 to {max}", max = crate::MAX_TAG_LEN)]
    BadTagLength(usize),
    /// Tags are not strictly ascending (unsorted or duplicated).
    #[error("tags are not sorted and unique")]
    TagsUnsorted,
    /// A record key is empty or longer than allowed.
    #[error("record key is {0} bytes, must be 1 to {max}", max = crate::MAX_RECORD_KEY_LEN)]
    BadRecordKeyLength(usize),
    /// A record value is larger than allowed.
    #[error("record value is {0} bytes, limit is {max}", max = crate::MAX_RECORD_VALUE_LEN)]
    RecordValueTooLarge(usize),
    /// Genesis admins are empty, unsorted, duplicated, too many, or omit the author.
    #[error("genesis admins must be sorted, unique, 1 to {max}, and include the author", max = crate::MAX_ADMINS)]
    BadAdmins,
    /// A threshold is zero, or (at genesis) larger than the admin count.
    #[error("invalid threshold {0}")]
    BadThreshold(u8),
    /// An invite allows zero uses.
    #[error("invite must allow at least one use")]
    ZeroMaxUses,
    /// A join proof's invite or device signature does not verify.
    #[error("invalid join proof")]
    BadJoinProof,
    /// The orphan buffer is full; the op was dropped. Fetch its ancestors first.
    #[error("orphan buffer full, limit is {max}", max = crate::MAX_ORPHANS)]
    OrphanBufferFull,
    /// A WireGuard client's public key is all zeros.
    #[error("invalid WireGuard client key")]
    BadClientKey,
}
