//! Ledger operations: their content, encoding, signatures and structural checks.
//!
//! An op exists in two forms:
//!
//! - [`UnsignedOp`] is plain data: what an author wants to say. It is checked when it is
//!   encoded or signed.
//! - [`SignedOp`] is the wire and storage form. It keeps the exact encoded bytes of the
//!   unsigned op plus the signatures over them. It can only be built by signing or by
//!   decoding, and both paths run every structural check, so holding a `SignedOp` means
//!   it is well-formed and every signature on it verifies.
//!
//! ```text
//! unsigned bytes = envelope(type 0x10, version 1, postcard(UnsignedOp))
//! OpId           = BLAKE3("kith 2026-10 ledger op id v1", unsigned bytes)
//! signature      = Ed25519 over the unsigned bytes, context "kith 2026-10 ledger op sig v1"
//! signed bytes   = envelope(type 0x11, version 1, postcard(unsigned bytes, signatures))
//! ```
//!
//! Signatures sit outside the hashed bytes, so co-signers can add theirs without changing
//! the op's identity. The postcard field and variant order below is the wire format:
//! reordering anything is a protocol break, which the golden-bytes test catches.

use alloc::string::String;
use alloc::vec::Vec;

use kith_crypto::{Context, Hash, Signature, SigningKey, VerifyingKey};
use kith_types::{Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId, OpId, Timestamp};
use kith_wire::envelope::{self, MessageType};
use kith_wire::registry;
use serde::{Deserialize, Serialize};

use crate::LedgerError;
use crate::join::JoinProof;
use crate::limits::{
    MAX_ADMINS, MAX_NAME_LEN, MAX_OP_LEN, MAX_PARENTS, MAX_RECORD_KEY_LEN, MAX_RECORD_VALUE_LEN,
    MAX_SIGNATURES, MAX_SIGNED_OP_LEN, MAX_TAG_LEN, MAX_TAGS,
};

/// Envelope type of the encoded [`UnsignedOp`]: `0x10`, from the central registry.
pub const UNSIGNED_OP_TYPE: MessageType = registry::LEDGER_UNSIGNED_OP;

/// Envelope type of the encoded [`SignedOp`]: `0x11`, from the central registry.
pub const SIGNED_OP_TYPE: MessageType = registry::LEDGER_SIGNED_OP;

/// Schema version of both op envelopes.
pub const OP_VERSION: u64 = 1;

/// Hashes the unsigned bytes into an [`OpId`].
pub const OP_ID_CONTEXT: Context = Context::new("kith 2026-10 ledger op id v1");

/// Signs the unsigned bytes.
pub const OP_SIGNATURE_CONTEXT: Context = Context::new("kith 2026-10 ledger op sig v1");

/// Network-wide settings fixed at genesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkParams {
    /// The IPv4 range device addresses are allocated from.
    pub ipv4: Ipv4Prefix,
    /// The IPv6 range device addresses are allocated from.
    pub ipv6: Ipv6Prefix,
}

/// What an op does.
///
/// Variants fall into four groups, which decide how the op is authorized (see the crate
/// documentation):
///
/// - [`Genesis`](Self::Genesis) founds the network.
/// - Authority ops ([`AddAdmin`](Self::AddAdmin), [`RemoveAdmin`](Self::RemoveAdmin),
///   [`SetThreshold`](Self::SetThreshold)) need a threshold of admin signatures.
/// - Admin ops need one current admin as author.
/// - [`Join`](Self::Join) needs a member device as sponsor and a valid invite.
///
/// The variant order is part of the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpBody {
    /// Founds the network. The author becomes the first member device.
    Genesis {
        /// Human-readable network name, at most [`MAX_NAME_LEN`] bytes.
        name: String,
        /// The initial admins: sorted, unique, 1 to [`MAX_ADMINS`], including the author.
        admins: Vec<NodeId>,
        /// Admin signatures needed for authority ops: 1 to `admins.len()`.
        threshold: u8,
        /// Network-wide settings.
        params: NetworkParams,
        /// The founding device's name, at most [`MAX_NAME_LEN`] bytes.
        founder_name: String,
    },
    /// Authority op: makes `admin` an admin.
    AddAdmin {
        /// The new admin.
        admin: NodeId,
    },
    /// Authority op: revokes `admin`'s admin role. Cannot drop the count below the threshold.
    RemoveAdmin {
        /// The admin to remove.
        admin: NodeId,
    },
    /// Authority op: changes the number of admin signatures authority ops need.
    SetThreshold {
        /// The new threshold: 1 to the admin count.
        threshold: u8,
    },
    /// Admin op: admits a device directly.
    AddDevice {
        /// The device's identity.
        device: NodeId,
        /// The device's name, at most [`MAX_NAME_LEN`] bytes.
        name: String,
        /// Sorted, unique tags; see [`MAX_TAGS`] and [`MAX_TAG_LEN`].
        tags: Vec<String>,
    },
    /// Admin op: removes a device.
    RemoveDevice {
        /// The device to remove.
        device: NodeId,
    },
    /// Admin op: creates an invite that devices redeem with a [`Join`](Self::Join).
    CreateInvite {
        /// The invite's public key. The secret half travels in the invite link.
        invite_key: VerifyingKey,
        /// The last instant a join may claim. Joins carry the sponsor's attested time.
        expires: Timestamp,
        /// How many joins the invite admits; at least one.
        max_uses: u16,
        /// Tags given to every device that joins with this invite.
        tags: Vec<String>,
    },
    /// Admin op: revokes an invite forever.
    RevokeInvite {
        /// The invite's public key.
        invite_key: VerifyingKey,
    },
    /// Admin op: sets or deletes one entry of the last-writer-wins key-value store.
    SetRecord {
        /// The key: 1 to [`MAX_RECORD_KEY_LEN`] bytes.
        key: String,
        /// The value, at most [`MAX_RECORD_VALUE_LEN`] bytes. `None` deletes the key.
        value: Option<Vec<u8>>,
    },
    /// Admits a device that redeemed an invite. The op's author is the sponsor.
    Join(JoinProof),
}

/// The authorization group an op body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Genesis,
    Authority,
    Admin,
    Join,
}

impl OpBody {
    pub(crate) const fn kind(&self) -> Kind {
        match self {
            Self::Genesis { .. } => Kind::Genesis,
            Self::AddAdmin { .. } | Self::RemoveAdmin { .. } | Self::SetThreshold { .. } => {
                Kind::Authority
            }
            Self::AddDevice { .. }
            | Self::RemoveDevice { .. }
            | Self::CreateInvite { .. }
            | Self::RevokeInvite { .. }
            | Self::SetRecord { .. } => Kind::Admin,
            Self::Join(_) => Kind::Join,
        }
    }

    /// Context-free checks on the body. `network` is `None` only for genesis.
    fn check(&self, author: NodeId, network: Option<NetworkId>) -> Result<(), LedgerError> {
        match self {
            Self::Genesis {
                name,
                admins,
                threshold,
                params: _,
                founder_name,
            } => {
                check_name(name)?;
                check_name(founder_name)?;
                if admins.is_empty()
                    || admins.len() > MAX_ADMINS
                    || !is_strictly_ascending(admins)
                    || admins.binary_search(&author).is_err()
                {
                    return Err(LedgerError::BadAdmins);
                }
                for admin in admins {
                    check_key(*admin)?;
                }
                if *threshold == 0 || usize::from(*threshold) > admins.len() {
                    return Err(LedgerError::BadThreshold(*threshold));
                }
                Ok(())
            }
            Self::AddAdmin { admin } => check_key(*admin),
            Self::RemoveAdmin { admin: _ }
            | Self::RemoveDevice { device: _ }
            | Self::RevokeInvite { invite_key: _ } => Ok(()),
            Self::SetThreshold { threshold } => {
                if *threshold == 0 {
                    return Err(LedgerError::BadThreshold(*threshold));
                }
                Ok(())
            }
            Self::AddDevice { device, name, tags } => {
                check_key(*device)?;
                check_name(name)?;
                check_tags(tags)
            }
            Self::CreateInvite {
                invite_key: _,
                expires: _,
                max_uses,
                tags,
            } => {
                if *max_uses == 0 {
                    return Err(LedgerError::ZeroMaxUses);
                }
                check_tags(tags)
            }
            Self::SetRecord { key, value } => {
                if key.is_empty() || key.len() > MAX_RECORD_KEY_LEN {
                    return Err(LedgerError::BadRecordKeyLength(key.len()));
                }
                match value {
                    Some(value) if value.len() > MAX_RECORD_VALUE_LEN => {
                        Err(LedgerError::RecordValueTooLarge(value.len()))
                    }
                    Some(_) | None => Ok(()),
                }
            }
            Self::Join(proof) => {
                // Genesis is the only op without a network, and it is not a join.
                let network = network.ok_or(LedgerError::MissingNetwork)?;
                proof.verify(network)
            }
        }
    }
}

/// An op before it is signed: plain data, checked when encoded or signed.
///
/// Use [`Ledger::draft`](crate::Ledger::draft) to fill in the network, parents and lamport
/// clock correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedOp {
    /// The network this op belongs to. `None` only for genesis, which defines the network.
    pub network: Option<NetworkId>,
    /// The DAG tips the author had seen. Genesis has none; every other op has 1 to
    /// [`MAX_PARENTS`], strictly ascending.
    pub parents: Vec<OpId>,
    /// Who wrote the op. Must be among its signers.
    pub author: NodeId,
    /// Exactly one more than the largest parent lamport. Genesis has 0.
    pub lamport: u64,
    /// The author's wall-clock time. Only invite expiry relies on it.
    pub timestamp: Timestamp,
    /// What the op does.
    pub body: OpBody,
}

impl UnsignedOp {
    /// Encodes the op into the exact bytes that are hashed and signed.
    ///
    /// Fails if the op is structurally invalid or too large.
    pub fn encode(&self) -> Result<Vec<u8>, LedgerError> {
        self.check()?;
        let bytes = envelope::encode(UNSIGNED_OP_TYPE, OP_VERSION, self)?;
        check_len(bytes.len(), MAX_OP_LEN)?;
        Ok(bytes)
    }

    /// Encodes and signs the op with the author's key.
    ///
    /// Fails if `key` is not the author's, or if the op is structurally invalid. Other
    /// signers add theirs with [`SignedOp::add_signature`].
    pub fn sign(&self, key: &SigningKey) -> Result<SignedOp, LedgerError> {
        if key.node_id() != self.author {
            return Err(LedgerError::AuthorNotSigner);
        }
        let bytes = self.encode()?;
        let signature = key.sign(OP_SIGNATURE_CONTEXT, &bytes);
        Ok(SignedOp {
            id: op_id(&bytes),
            unsigned_bytes: bytes,
            op: self.clone(),
            signatures: alloc::vec![(self.author, signature)],
        })
    }

    /// Decodes and checks unsigned bytes.
    fn decode(bytes: &[u8]) -> Result<Self, LedgerError> {
        check_len(bytes.len(), MAX_OP_LEN)?;
        let (version, op) = envelope::decode::<Self>(bytes, UNSIGNED_OP_TYPE, OP_VERSION)?;
        if version != OP_VERSION {
            return Err(LedgerError::UnsupportedVersion(version));
        }
        op.check()?;
        Ok(op)
    }

    /// Every check that needs nothing but the op itself.
    fn check(&self) -> Result<(), LedgerError> {
        if self.body.kind() == Kind::Genesis {
            if self.network.is_some() || !self.parents.is_empty() || self.lamport != 0 {
                return Err(LedgerError::MalformedGenesis);
            }
        } else {
            if self.network.is_none() {
                return Err(LedgerError::MissingNetwork);
            }
            if self.parents.is_empty() {
                return Err(LedgerError::NoParents);
            }
            if self.parents.len() > MAX_PARENTS {
                return Err(LedgerError::TooManyParents(self.parents.len()));
            }
            if !is_strictly_ascending(&self.parents) {
                return Err(LedgerError::ParentsUnsorted);
            }
            // The exact value needs the parents; at least it cannot be genesis's.
            if self.lamport == 0 {
                return Err(LedgerError::WrongLamport {
                    expected: 1,
                    found: 0,
                });
            }
        }
        self.body.check(self.author, self.network)
    }
}

/// A well-formed op with verified signatures: the wire and storage form.
///
/// Invariants, established by every constructor:
///
/// - `unsigned_bytes` decode to a structurally valid [`UnsignedOp`], kept alongside;
/// - 1 to [`MAX_SIGNATURES`] signatures, strictly ascending by signer, each verifying
///   over `unsigned_bytes`;
/// - the author is among the signers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedOp {
    id: OpId,
    unsigned_bytes: Vec<u8>,
    op: UnsignedOp,
    signatures: Vec<(NodeId, Signature)>,
}

/// The encoded body of a [`SignedOp`]. Field order is the wire format.
#[derive(Serialize, Deserialize)]
struct SignedOpWire {
    unsigned_bytes: Vec<u8>,
    signatures: Vec<(NodeId, Signature)>,
}

impl SignedOp {
    /// Decodes an op received from the network or storage, running every structural check
    /// and verifying every signature over the exact bytes received.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LedgerError> {
        check_len(bytes.len(), MAX_SIGNED_OP_LEN)?;
        let (version, wire) = envelope::decode::<SignedOpWire>(bytes, SIGNED_OP_TYPE, OP_VERSION)?;
        if version != OP_VERSION {
            return Err(LedgerError::UnsupportedVersion(version));
        }
        let op = UnsignedOp::decode(&wire.unsigned_bytes)?;
        let signatures = wire.signatures;
        if signatures.is_empty() {
            return Err(LedgerError::NoSignatures);
        }
        if signatures.len() > MAX_SIGNATURES {
            return Err(LedgerError::TooManySignatures);
        }
        if !signatures
            .windows(2)
            .all(|pair| matches!(pair, [(a, _), (b, _)] if a < b))
        {
            return Err(LedgerError::SignaturesUnsorted);
        }
        for (signer, signature) in &signatures {
            verify(*signer, &wire.unsigned_bytes, signature)?;
        }
        if signatures
            .binary_search_by(|(signer, _)| signer.cmp(&op.author))
            .is_err()
        {
            return Err(LedgerError::AuthorNotSigner);
        }
        Ok(Self {
            id: op_id(&wire.unsigned_bytes),
            unsigned_bytes: wire.unsigned_bytes,
            op,
            signatures,
        })
    }

    /// Encodes the op for the network or storage.
    ///
    /// # Panics
    ///
    /// Never: a valid `SignedOp` is far below the envelope size limit.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let wire = SignedOpWire {
            unsigned_bytes: self.unsigned_bytes.clone(),
            signatures: self.signatures.clone(),
        };
        #[expect(
            clippy::expect_used,
            reason = "postcard into a Vec is infallible for these types, and a SignedOp is \
                      bounded by MAX_SIGNED_OP_LEN, far below the envelope limit"
        )]
        envelope::encode(SIGNED_OP_TYPE, OP_VERSION, &wire)
            .expect("a valid SignedOp always encodes")
    }

    /// Adds a co-signature, for authority ops that need several admins.
    ///
    /// Signing twice with the same key is a no-op. Fails if the op already carries
    /// [`MAX_SIGNATURES`] signatures.
    pub fn add_signature(&mut self, key: &SigningKey) -> Result<(), LedgerError> {
        let signer = key.node_id();
        match self.signatures.binary_search_by(|(id, _)| id.cmp(&signer)) {
            Ok(_) => Ok(()),
            Err(_) if self.signatures.len() >= MAX_SIGNATURES => {
                Err(LedgerError::TooManySignatures)
            }
            Err(position) => {
                let signature = key.sign(OP_SIGNATURE_CONTEXT, &self.unsigned_bytes);
                self.signatures.insert(position, (signer, signature));
                Ok(())
            }
        }
    }

    /// Merges another copy's signatures into this one. Returns whether anything changed.
    ///
    /// Both copies must have the same id. If the union exceeds [`MAX_SIGNATURES`], the
    /// author's signature and the lowest other signers are kept. That choice depends only
    /// on the union, so replicas converge whatever order the copies arrive in.
    pub(crate) fn merge_signatures(&mut self, other: &Self) -> bool {
        let mut merged = self.signatures.clone();
        for entry in &other.signatures {
            if let Err(position) = merged.binary_search_by(|(id, _)| id.cmp(&entry.0)) {
                merged.insert(position, *entry);
            }
        }
        let author = self.op.author;
        while merged.len() > MAX_SIGNATURES {
            // Drop the highest signer that is not the author.
            if let Some(position) = merged.iter().rposition(|(id, _)| *id != author) {
                merged.remove(position);
            } else {
                break;
            }
        }
        let changed = merged != self.signatures;
        self.signatures = merged;
        changed
    }

    /// The op's identity: the hash of its unsigned bytes.
    pub const fn id(&self) -> OpId {
        self.id
    }

    /// The decoded content.
    pub const fn op(&self) -> &UnsignedOp {
        &self.op
    }

    /// The exact bytes that were hashed and signed.
    pub fn unsigned_bytes(&self) -> &[u8] {
        &self.unsigned_bytes
    }

    /// The signatures, ascending by signer.
    pub fn signatures(&self) -> &[(NodeId, Signature)] {
        &self.signatures
    }

    /// The signers, ascending.
    pub fn signers(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.signatures.iter().map(|(signer, _)| *signer)
    }
}

/// The [`OpId`] of encoded unsigned bytes.
fn op_id(unsigned_bytes: &[u8]) -> OpId {
    OpId::from_bytes(Hash::of(OP_ID_CONTEXT, unsigned_bytes).to_bytes())
}

fn verify(signer: NodeId, bytes: &[u8], signature: &Signature) -> Result<(), LedgerError> {
    VerifyingKey::from_node_id(signer)
        .map_err(|_| LedgerError::InvalidKey(signer))?
        .verify(OP_SIGNATURE_CONTEXT, bytes, signature)
        .map_err(|_| LedgerError::BadSignature(signer))
}

fn check_len(len: usize, max: usize) -> Result<(), LedgerError> {
    if len > max {
        return Err(LedgerError::TooLarge { len, max });
    }
    Ok(())
}

fn check_key(id: NodeId) -> Result<(), LedgerError> {
    VerifyingKey::from_node_id(id)
        .map(|_| ())
        .map_err(|_| LedgerError::InvalidKey(id))
}

pub(crate) const fn check_name(name: &str) -> Result<(), LedgerError> {
    if name.len() > MAX_NAME_LEN {
        return Err(LedgerError::NameTooLong(name.len()));
    }
    Ok(())
}

fn check_tags(tags: &[String]) -> Result<(), LedgerError> {
    if tags.len() > MAX_TAGS {
        return Err(LedgerError::TooManyTags(tags.len()));
    }
    if let Some(tag) = tags
        .iter()
        .find(|tag| tag.is_empty() || tag.len() > MAX_TAG_LEN)
    {
        return Err(LedgerError::BadTagLength(tag.len()));
    }
    if !is_strictly_ascending(tags) {
        return Err(LedgerError::TagsUnsorted);
    }
    Ok(())
}

fn is_strictly_ascending<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| matches!(pair, [a, b] if a < b))
}
