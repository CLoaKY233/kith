//! Ledger operations: their content, encoding, signatures and structural checks.
//!
//! [`UnsignedOp`] is plain data. [`SignedOp`] is the wire and storage form; it can only be
//! built by signing or decoding, both of which run every structural check, so holding one
//! means it is well-formed and every signature verifies.
//!
//! ```text
//! unsigned bytes = envelope(type 0x10, version 1, postcard(UnsignedOp))
//! OpId           = BLAKE3("kith 2026-10 ledger op id v1", unsigned bytes)
//! signature      = Ed25519 over the unsigned bytes, context "kith 2026-10 ledger op sig v1"
//! signed bytes   = envelope(type 0x11, version 1, postcard(unsigned bytes, signatures))
//! ```
//!
//! The postcard field and variant order below is the wire format: reordering is a protocol
//! break, which the golden-bytes test catches.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use kith_crypto::{Context, Hash, Signature, SigningKey, VerifyingKey};
use kith_types::{AccountId, Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId, OpId, Timestamp};
use kith_wire::envelope::{self, MessageType};
use kith_wire::registry;
use serde::{Deserialize, Serialize};

use crate::LedgerError;
use crate::account::{AccountChange, AdminChange, Approval, Credential, Role};
use crate::join::JoinProof;
use crate::limits::{
    MAX_ADMINS, MAX_NAME_LEN, MAX_OP_LEN, MAX_PARENTS, MAX_RECORD_KEY_LEN, MAX_RECORD_VALUE_LEN,
    MAX_SIGNATURES, MAX_SIGNED_OP_LEN, MAX_TAG_LEN, MAX_TAGS,
};

/// Envelope type of the encoded [`UnsignedOp`]: `0x10`, from the central registry.
pub(crate) const UNSIGNED_OP_TYPE: MessageType = registry::LEDGER_UNSIGNED_OP;

/// Envelope type of the encoded [`SignedOp`]: `0x11`, from the central registry.
pub(crate) const SIGNED_OP_TYPE: MessageType = registry::LEDGER_SIGNED_OP;

/// Schema version of both op envelopes.
pub(crate) const OP_VERSION: u64 = 1;

/// Hashes the unsigned bytes into an [`OpId`].
pub(crate) const OP_ID_CONTEXT: Context = Context::new("kith 2026-10 ledger op id v1");

/// Signs the unsigned bytes.
pub(crate) const OP_SIGNATURE_CONTEXT: Context = Context::new("kith 2026-10 ledger op sig v1");

/// Network-wide settings fixed at genesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkParams {
    /// The IPv4 range device addresses are allocated from.
    pub ipv4: Ipv4Prefix,
    /// The IPv6 range device addresses are allocated from.
    pub ipv6: Ipv6Prefix,
}

/// What an op does. Authority ops need a threshold of admin signatures, admin ops one
/// admin author (or an authorized console acting for a person), joins a member sponsor plus
/// a valid invite, and account ops a person's passkey or the recovery key. Variant order is
/// wire format.
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
    /// Admin op: admits a device that runs plain WireGuard instead of kith, such as a
    /// phone. It is a member like any other, reached through `gateway`, and its id is
    /// [`client_id`] of `key`. Remove it with [`RemoveDevice`](Self::RemoveDevice).
    AddClient {
        /// Its WireGuard (X25519) public key.
        key: [u8; 32],
        /// The kith device it connects to.
        gateway: NodeId,
        /// The device's name, at most [`MAX_NAME_LEN`] bytes.
        name: String,
        /// Sorted, unique tags; see [`MAX_TAGS`] and [`MAX_TAG_LEN`].
        tags: Vec<String>,
        /// Its IPv4 address, fixed for good: it is written into the device's config.
        v4: Ipv4Addr,
        /// Its IPv6 address, fixed for good.
        v6: Ipv6Addr,
    },
    /// Admin op: renames a member device. The latest rename after its admission wins.
    RenameDevice {
        /// The device to rename.
        device: NodeId,
        /// Its new name, at most [`MAX_NAME_LEN`] bytes.
        name: String,
    },
    /// Account op: adds a person, whose [`AccountId`] is [`Credential::account_id`]. Valid
    /// plain only as a network's first account, with role owner, approved like an
    /// authority op by its admin devices; otherwise it must be [`Approved`](Self::Approved)
    /// by an owner, an admin (for roles below owner) or the recovery key.
    AddAccount {
        /// The person's name, at most [`MAX_NAME_LEN`] bytes.
        name: String,
        /// Their role.
        role: Role,
        /// Their first passkey.
        credential: Credential,
    },
    /// Account op, valid only [`Approved`](Self::Approved) by an owner, or by an admin for
    /// a non-owner: removes a person and every device assigned to them. The last owner
    /// cannot be removed.
    RemoveAccount {
        /// The account.
        account: AccountId,
    },
    /// Account op, valid only [`Approved`](Self::Approved) by an owner: changes a role.
    /// The last owner cannot be demoted.
    SetRole {
        /// The account.
        account: AccountId,
        /// Its new role.
        role: Role,
    },
    /// Admin op: records whose device `device` is. The latest assignment after the
    /// device's admission wins.
    AssignDevice {
        /// The device.
        device: NodeId,
        /// The person it belongs to; must exist when the op is made.
        account: AccountId,
    },
    /// Account op, valid only [`Approved`](Self::Approved) by an owner or the recovery key:
    /// lets `device` sign admin ops that are [`Acted`](Self::Acted) for a person.
    AuthorizeConsole {
        /// The console's device.
        device: NodeId,
    },
    /// Account op, valid only [`Approved`](Self::Approved) by an owner or an admin: takes
    /// the console's power away.
    RevokeConsole {
        /// The console's device.
        device: NodeId,
    },
    /// Account op: sets the recovery key, whose approval counts as an owner's for adding
    /// people and authorizing consoles. Valid only while the network has neither accounts
    /// nor a recovery key, approved like an authority op by its admin devices.
    SetRecoveryKey {
        /// The recovery key's public half.
        key: VerifyingKey,
    },
    /// Account op: a change to people or consoles with the approval it needs. The
    /// approval signs the challenge: the [`OpId`] of this op with
    /// [`AccountChange::to_body`] as its body (see [`UnsignedOp::challenge`]).
    Approved {
        /// Who approved, over the challenge.
        approval: Approval,
        /// The change.
        change: AccountChange,
    },
    /// Admin op made for a person: valid when the author is an admin device or an
    /// authorized console, and `actor` holds an owner or admin role, at that point.
    Acted {
        /// The person who asked for the change.
        actor: AccountId,
        /// The change.
        change: AdminChange,
    },
    /// A device leaves: removes its own author from the network, as a `RemoveDevice` of
    /// itself would. Any device may sign it, for itself only.
    Leave,
    /// Account op, valid only [`Approved`](Self::Approved) by the account's own passkey or
    /// an owner: adds another passkey to an existing person.
    AddCredential {
        /// The account.
        account: AccountId,
        /// The new passkey, at the same relying party as the others or not.
        credential: Credential,
    },
}

/// The hash context that turns a WireGuard key into a member id.
const CLIENT_ID_CONTEXT: Context = Context::new("kith 2026-10 wireguard client id v1");

/// The member id of a plain WireGuard device with public `key`. It is a hash, so it is
/// never anyone's signing key and the device can never author an op.
pub fn client_id(key: &[u8; 32]) -> NodeId {
    NodeId::from_bytes(Hash::of(CLIENT_ID_CONTEXT, key).to_bytes())
}

/// The authorization group an op body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Genesis,
    Authority,
    Admin,
    Join,
    Account,
    Leave,
}

impl OpBody {
    /// Whether this is an everyday admin change (devices, invites, records, names), the
    /// kind a console may make only on behalf of a person.
    pub const fn is_admin_change(&self) -> bool {
        matches!(self.kind(), Kind::Admin)
    }

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
            | Self::SetRecord { .. }
            | Self::AddClient { .. }
            | Self::RenameDevice { .. }
            | Self::AssignDevice { .. }
            | Self::Acted { .. } => Kind::Admin,
            Self::Join(_) => Kind::Join,
            Self::Leave => Kind::Leave,
            Self::AddAccount { .. }
            | Self::RemoveAccount { .. }
            | Self::SetRole { .. }
            | Self::AuthorizeConsole { .. }
            | Self::RevokeConsole { .. }
            | Self::SetRecoveryKey { .. }
            | Self::Approved { .. }
            | Self::AddCredential { .. } => Kind::Account,
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
            | Self::RevokeInvite { invite_key: _ }
            | Self::Leave => Ok(()),
            Self::AddAccount { .. }
            | Self::RemoveAccount { .. }
            | Self::SetRole { .. }
            | Self::AssignDevice { .. }
            | Self::AuthorizeConsole { .. }
            | Self::RevokeConsole { .. }
            | Self::SetRecoveryKey { .. }
            | Self::Approved { .. }
            | Self::Acted { .. }
            | Self::AddCredential { .. } => self.check_account(author, network),
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
                let network = network.ok_or(LedgerError::MissingNetwork)?;
                proof.verify(network)
            }
            Self::AddClient {
                key,
                gateway,
                name,
                tags,
                v4: _,
                v6: _,
            } => {
                if key.iter().all(|byte| *byte == 0) {
                    return Err(LedgerError::BadClientKey);
                }
                check_key(*gateway)?;
                check_name(name)?;
                check_tags(tags)
            }
            Self::RenameDevice { device: _, name } => check_name(name),
        }
    }

    /// [`check`](Self::check) for the ops about people. A wrapped change gets its own
    /// check; it cannot itself be a wrapper.
    fn check_account(&self, author: NodeId, network: Option<NetworkId>) -> Result<(), LedgerError> {
        match self {
            Self::AddAccount {
                name,
                role: _,
                credential,
            } => {
                check_name(name)?;
                credential.check()
            }
            Self::AuthorizeConsole { device } => check_key(*device),
            Self::AddCredential {
                account: _,
                credential,
            } => credential.check(),
            Self::Approved { approval, change } => {
                approval.check()?;
                change.to_body().check(author, network)
            }
            Self::Acted { actor: _, change } => change.to_body().check(author, network),
            Self::RemoveAccount { account: _ }
            | Self::SetRole {
                account: _,
                role: _,
            }
            | Self::AssignDevice {
                device: _,
                account: _,
            }
            | Self::RevokeConsole { device: _ }
            | Self::SetRecoveryKey { key: _ }
            | Self::Genesis { .. }
            | Self::AddAdmin { .. }
            | Self::RemoveAdmin { .. }
            | Self::SetThreshold { .. }
            | Self::AddDevice { .. }
            | Self::RemoveDevice { .. }
            | Self::CreateInvite { .. }
            | Self::RevokeInvite { .. }
            | Self::SetRecord { .. }
            | Self::Join(_)
            | Self::AddClient { .. }
            | Self::RenameDevice { .. }
            | Self::Leave => Ok(()),
        }
    }

    /// The plain change inside a wrapper op ([`Approved`](Self::Approved) or
    /// [`Acted`](Self::Acted)); `None` for every other op.
    fn wrapped(&self) -> Option<Self> {
        match self {
            Self::Approved { change, .. } => Some(change.to_body()),
            Self::Acted { change, .. } => Some(change.to_body()),
            Self::Genesis { .. }
            | Self::AddAdmin { .. }
            | Self::RemoveAdmin { .. }
            | Self::SetThreshold { .. }
            | Self::AddDevice { .. }
            | Self::RemoveDevice { .. }
            | Self::CreateInvite { .. }
            | Self::RevokeInvite { .. }
            | Self::SetRecord { .. }
            | Self::Join(_)
            | Self::AddClient { .. }
            | Self::RenameDevice { .. }
            | Self::AddAccount { .. }
            | Self::RemoveAccount { .. }
            | Self::SetRole { .. }
            | Self::AssignDevice { .. }
            | Self::AuthorizeConsole { .. }
            | Self::RevokeConsole { .. }
            | Self::SetRecoveryKey { .. }
            | Self::AddCredential { .. }
            | Self::Leave => None,
        }
    }
}

/// An op before it is signed. [`Ledger::draft`](crate::Ledger::draft) fills in the network,
/// parents and lamport clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedOp {
    /// The network this op belongs to. `None` only for genesis, which defines the network.
    pub network: Option<NetworkId>,
    /// The DAG tips the author had seen: 1 to [`MAX_PARENTS`], strictly ascending; none for genesis.
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
    pub fn encode(&self) -> Result<Vec<u8>, LedgerError> {
        self.check()?;
        let bytes = envelope::encode(UNSIGNED_OP_TYPE, OP_VERSION, self)?;
        check_len(bytes.len(), MAX_OP_LEN)?;
        Ok(bytes)
    }

    /// Encodes and signs the op with the author's key; fails if `key` is not the author's.
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
            derived: Derived::of(self)?,
        })
    }

    /// Decodes an op from [`encode`](Self::encode)'s bytes, running every structural check.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LedgerError> {
        Self::decode(bytes)
    }

    /// The [`OpId`] the op has once signed.
    pub fn id(&self) -> Result<OpId, LedgerError> {
        self.encode().map(|bytes| op_id(&bytes))
    }

    /// For an [`OpBody::Approved`] op, what its approval must sign: the [`OpId`] of the
    /// same op with the plain change as its body. `None` for every other op.
    pub fn challenge(&self) -> Result<Option<OpId>, LedgerError> {
        let OpBody::Approved { change, .. } = &self.body else {
            return Ok(None);
        };
        let plain = Self {
            body: change.to_body(),
            ..self.clone()
        };
        plain.id().map(Some)
    }

    /// Wraps a plain account op (see [`AccountChange`]) in an approval of its
    /// [`id`](Self::id), which becomes the wrapped op's [`challenge`](Self::challenge).
    pub fn approve(self, approval: Approval) -> Result<Self, LedgerError> {
        let change = AccountChange::from_body(self.body).ok_or(LedgerError::NotWrappable)?;
        Ok(Self {
            body: OpBody::Approved { approval, change },
            ..self
        })
    }

    /// Wraps a plain admin op (see [`AdminChange`]) as made for `actor`.
    pub fn act_for(self, actor: AccountId) -> Result<Self, LedgerError> {
        let change = AdminChange::from_body(self.body).ok_or(LedgerError::NotWrappable)?;
        Ok(Self {
            body: OpBody::Acted { actor, change },
            ..self
        })
    }

    fn decode(bytes: &[u8]) -> Result<Self, LedgerError> {
        check_len(bytes.len(), MAX_OP_LEN)?;
        let (version, op) = envelope::decode::<Self>(bytes, UNSIGNED_OP_TYPE, OP_VERSION)?;
        if version != OP_VERSION {
            return Err(LedgerError::UnsupportedVersion(version));
        }
        op.check()?;
        Ok(op)
    }

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
/// Invariants: `unsigned_bytes` decode to `op`; 1 to [`MAX_SIGNATURES`] signatures,
/// strictly ascending by signer, each verifying over `unsigned_bytes`; the author signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedOp {
    id: OpId,
    unsigned_bytes: Vec<u8>,
    op: UnsignedOp,
    signatures: Vec<(NodeId, Signature)>,
    derived: Derived,
}

/// What a [`SignedOp`] computes once from its content, so validity can borrow it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Derived {
    /// The plain change inside a wrapper op.
    wrapped: Option<OpBody>,
    /// What an approval signed.
    challenge: Option<OpId>,
}

impl Derived {
    fn of(op: &UnsignedOp) -> Result<Self, LedgerError> {
        Ok(Self {
            wrapped: op.body.wrapped(),
            challenge: op.challenge()?,
        })
    }
}

/// The encoded body of a [`SignedOp`]. Field order is the wire format.
#[derive(Serialize, Deserialize)]
struct SignedOpWire {
    unsigned_bytes: Vec<u8>,
    signatures: Vec<(NodeId, Signature)>,
}

impl SignedOp {
    /// Decodes an op, running every structural check and verifying every signature.
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
            derived: Derived::of(&op)?,
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

    /// Adds a co-signature; signing twice with one key is a no-op.
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
    /// Over [`MAX_SIGNATURES`], the author and the lowest other signers are kept; that
    /// depends only on the union, so replicas converge.
    pub(crate) fn merge_signatures(&mut self, other: &Self) -> bool {
        let mut merged = self.signatures.clone();
        for entry in &other.signatures {
            if let Err(position) = merged.binary_search_by(|(id, _)| id.cmp(&entry.0)) {
                merged.insert(position, *entry);
            }
        }
        let author = self.op.author;
        while merged.len() > MAX_SIGNATURES {
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

    /// What the op changes: the plain change inside an [`OpBody::Approved`] or
    /// [`OpBody::Acted`], and the body itself otherwise.
    pub fn effect(&self) -> &OpBody {
        self.derived.wrapped.as_ref().unwrap_or(&self.op.body)
    }

    /// For an [`OpBody::Approved`] op, what its approval signed; see
    /// [`UnsignedOp::challenge`].
    pub const fn challenge(&self) -> Option<OpId> {
        self.derived.challenge
    }
}

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
