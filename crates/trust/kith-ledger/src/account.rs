//! People in the ledger: accounts, their passkeys, and the approvals they sign.
//!
//! An account is a person with a [`Role`] and one or more passkeys. Changes to people and
//! consoles ([`AccountChange`]) are approved by an account's passkey or by the recovery
//! key, over a *challenge*: the [`OpId`] the same op would have with the plain change as
//! its body. Everyday admin changes ([`AdminChange`]) made through a console name the
//! person who asked.
//!
//! The wrapped changes are their own enums rather than a boxed [`OpBody`], so decoding
//! never recurses: a hostile op cannot nest wrappers until the decoder's stack runs out.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use kith_crypto::webauthn::{PasskeyError, PasskeyKey};
use kith_crypto::{Context, Hash, Signature, SigningKey, VerifyingKey};
use kith_types::{AccountId, NodeId, OpId, Timestamp};
use serde::{Deserialize, Serialize};

use crate::limits::{
    MAX_AUTHENTICATOR_DATA_LEN, MAX_CLIENT_DATA_LEN, MAX_CREDENTIAL_ID_LEN, MAX_PASSKEY_SIG_LEN,
    MAX_RP_ID_LEN,
};
use crate::{LedgerError, OpBody};

/// Hashes a passkey's public key into an [`AccountId`].
const ACCOUNT_ID_CONTEXT: Context = Context::new("kith 2026-11 ledger account id v1");

/// The recovery key signs challenges under this context.
pub(crate) const RECOVERY_CONTEXT: Context = Context::new("kith 2026-11 ledger recovery v1");

/// What a person may do. Variant order is wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Everything, including people's roles and consoles.
    Owner,
    /// Everyday changes, and adding people other than owners.
    Admin,
    /// Uses the network; changes nothing.
    Member,
}

impl Role {
    /// Whether the role may make everyday admin changes.
    pub const fn can_administer(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

/// The account a passkey with this public key founds.
pub fn account_id(public_key: &[u8]) -> AccountId {
    AccountId::from_bytes(Hash::of(ACCOUNT_ID_CONTEXT, public_key).to_bytes())
}

/// A passkey registered to an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// The authenticator's credential id: 1 to [`MAX_CREDENTIAL_ID_LEN`] bytes.
    pub id: Vec<u8>,
    /// The P-256 public key, as an uncompressed SEC1 point (65 bytes).
    pub public_key: Vec<u8>,
    /// The WebAuthn relying party id it was created for, such as `vpn.example.com`.
    pub rp_id: String,
}

impl Credential {
    /// The account this passkey founds when it is an account's first.
    pub fn account_id(&self) -> AccountId {
        account_id(&self.public_key)
    }

    pub(crate) fn check(&self) -> Result<(), LedgerError> {
        if self.id.is_empty()
            || self.id.len() > MAX_CREDENTIAL_ID_LEN
            || self.rp_id.is_empty()
            || self.rp_id.len() > MAX_RP_ID_LEN
        {
            return Err(LedgerError::BadCredential);
        }
        PasskeyKey::from_sec1(&self.public_key).map_err(|_| LedgerError::BadCredential)?;
        Ok(())
    }
}

/// A WebAuthn assertion: an account's passkey signing a challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    /// The account whose passkey signed.
    pub account: AccountId,
    /// Which of its passkeys.
    pub credential_id: Vec<u8>,
    /// The authenticator data, as the authenticator returned it.
    pub authenticator_data: Vec<u8>,
    /// The client data JSON, as the browser returned it.
    pub client_data_json: Vec<u8>,
    /// The ES256 signature, DER-encoded.
    pub signature: Vec<u8>,
}

impl Assertion {
    /// Checks that `credential` signed `challenge`: see [`kith_crypto::webauthn`].
    pub fn verify(&self, credential: &Credential, challenge: &OpId) -> Result<(), PasskeyError> {
        PasskeyKey::from_sec1(&credential.public_key)?.verify_assertion(
            &credential.rp_id,
            challenge.as_bytes(),
            &self.authenticator_data,
            &self.client_data_json,
            &self.signature,
        )
    }

    fn check(&self) -> Result<(), LedgerError> {
        if self.credential_id.is_empty()
            || self.credential_id.len() > MAX_CREDENTIAL_ID_LEN
            || self.authenticator_data.len() > MAX_AUTHENTICATOR_DATA_LEN
            || self.client_data_json.len() > MAX_CLIENT_DATA_LEN
            || self.signature.len() > MAX_PASSKEY_SIG_LEN
        {
            return Err(LedgerError::BadAssertion);
        }
        Ok(())
    }
}

/// Who approved an [`AccountChange`]. Variant order is wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Approval {
    /// An account's passkey.
    Passkey(Assertion),
    /// The network's recovery key, which counts as an owner for adding people and
    /// authorizing consoles.
    Recovery(Signature),
}

impl Approval {
    /// The recovery key's approval of `challenge`.
    pub fn recovery(key: &SigningKey, challenge: &OpId) -> Self {
        Self::Recovery(key.sign(RECOVERY_CONTEXT, challenge.as_bytes()))
    }

    pub(crate) fn verify_recovery(
        key: &VerifyingKey,
        challenge: &OpId,
        signature: &Signature,
    ) -> bool {
        key.verify(RECOVERY_CONTEXT, challenge.as_bytes(), signature)
            .is_ok()
    }

    pub(crate) fn check(&self) -> Result<(), LedgerError> {
        match self {
            Self::Passkey(assertion) => assertion.check(),
            Self::Recovery(_) => Ok(()),
        }
    }
}

/// A change to people or consoles, approved by a passkey or the recovery key in an
/// [`OpBody::Approved`]. Each is the same change as the [`OpBody`] variant of its name.
/// Variant order is wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountChange {
    /// See [`OpBody::AddAccount`].
    AddAccount {
        /// The person's name.
        name: String,
        /// Their role.
        role: Role,
        /// Their first passkey, whose key names the account.
        credential: Credential,
    },
    /// See [`OpBody::RemoveAccount`].
    RemoveAccount {
        /// The account.
        account: AccountId,
    },
    /// See [`OpBody::SetRole`].
    SetRole {
        /// The account.
        account: AccountId,
        /// Its new role.
        role: Role,
    },
    /// See [`OpBody::AuthorizeConsole`].
    AuthorizeConsole {
        /// The console's device.
        device: NodeId,
    },
    /// See [`OpBody::RevokeConsole`].
    RevokeConsole {
        /// The console's device.
        device: NodeId,
    },
}

impl AccountChange {
    /// The plain op body making the same change.
    pub fn to_body(&self) -> OpBody {
        match self.clone() {
            Self::AddAccount {
                name,
                role,
                credential,
            } => OpBody::AddAccount {
                name,
                role,
                credential,
            },
            Self::RemoveAccount { account } => OpBody::RemoveAccount { account },
            Self::SetRole { account, role } => OpBody::SetRole { account, role },
            Self::AuthorizeConsole { device } => OpBody::AuthorizeConsole { device },
            Self::RevokeConsole { device } => OpBody::RevokeConsole { device },
        }
    }

    /// The change a plain op body makes, if it is one an approval can carry.
    pub fn from_body(body: OpBody) -> Option<Self> {
        Some(match body {
            OpBody::AddAccount {
                name,
                role,
                credential,
            } => Self::AddAccount {
                name,
                role,
                credential,
            },
            OpBody::RemoveAccount { account } => Self::RemoveAccount { account },
            OpBody::SetRole { account, role } => Self::SetRole { account, role },
            OpBody::AuthorizeConsole { device } => Self::AuthorizeConsole { device },
            OpBody::RevokeConsole { device } => Self::RevokeConsole { device },
            OpBody::Genesis { .. }
            | OpBody::AddAdmin { .. }
            | OpBody::RemoveAdmin { .. }
            | OpBody::SetThreshold { .. }
            | OpBody::AddDevice { .. }
            | OpBody::RemoveDevice { .. }
            | OpBody::CreateInvite { .. }
            | OpBody::RevokeInvite { .. }
            | OpBody::SetRecord { .. }
            | OpBody::Join(_)
            | OpBody::AddClient { .. }
            | OpBody::RenameDevice { .. }
            | OpBody::AssignDevice { .. }
            | OpBody::SetRecoveryKey { .. }
            | OpBody::Approved { .. }
            | OpBody::Acted { .. } => return None,
        })
    }
}

/// An everyday admin change made on behalf of a person, in an [`OpBody::Acted`]. Each is
/// the same change as the [`OpBody`] variant of its name. Variant order is wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    missing_docs,
    reason = "each field is documented on the OpBody variant"
)]
pub enum AdminChange {
    /// See [`OpBody::AddDevice`].
    AddDevice {
        device: NodeId,
        name: String,
        tags: Vec<String>,
    },
    /// See [`OpBody::RemoveDevice`].
    RemoveDevice { device: NodeId },
    /// See [`OpBody::CreateInvite`].
    CreateInvite {
        invite_key: VerifyingKey,
        expires: Timestamp,
        max_uses: u16,
        tags: Vec<String>,
    },
    /// See [`OpBody::RevokeInvite`].
    RevokeInvite { invite_key: VerifyingKey },
    /// See [`OpBody::SetRecord`].
    SetRecord { key: String, value: Option<Vec<u8>> },
    /// See [`OpBody::AddClient`].
    AddClient {
        key: [u8; 32],
        gateway: NodeId,
        name: String,
        tags: Vec<String>,
        v4: Ipv4Addr,
        v6: Ipv6Addr,
    },
    /// See [`OpBody::RenameDevice`].
    RenameDevice { device: NodeId, name: String },
    /// See [`OpBody::AssignDevice`].
    AssignDevice { device: NodeId, account: AccountId },
}

impl AdminChange {
    /// The plain op body making the same change.
    pub fn to_body(&self) -> OpBody {
        match self.clone() {
            Self::AddDevice { device, name, tags } => OpBody::AddDevice { device, name, tags },
            Self::RemoveDevice { device } => OpBody::RemoveDevice { device },
            Self::CreateInvite {
                invite_key,
                expires,
                max_uses,
                tags,
            } => OpBody::CreateInvite {
                invite_key,
                expires,
                max_uses,
                tags,
            },
            Self::RevokeInvite { invite_key } => OpBody::RevokeInvite { invite_key },
            Self::SetRecord { key, value } => OpBody::SetRecord { key, value },
            Self::AddClient {
                key,
                gateway,
                name,
                tags,
                v4,
                v6,
            } => OpBody::AddClient {
                key,
                gateway,
                name,
                tags,
                v4,
                v6,
            },
            Self::RenameDevice { device, name } => OpBody::RenameDevice { device, name },
            Self::AssignDevice { device, account } => OpBody::AssignDevice { device, account },
        }
    }

    /// The change a plain op body makes, if it is an everyday admin change.
    pub fn from_body(body: OpBody) -> Option<Self> {
        Some(match body {
            OpBody::AddDevice { device, name, tags } => Self::AddDevice { device, name, tags },
            OpBody::RemoveDevice { device } => Self::RemoveDevice { device },
            OpBody::CreateInvite {
                invite_key,
                expires,
                max_uses,
                tags,
            } => Self::CreateInvite {
                invite_key,
                expires,
                max_uses,
                tags,
            },
            OpBody::RevokeInvite { invite_key } => Self::RevokeInvite { invite_key },
            OpBody::SetRecord { key, value } => Self::SetRecord { key, value },
            OpBody::AddClient {
                key,
                gateway,
                name,
                tags,
                v4,
                v6,
            } => Self::AddClient {
                key,
                gateway,
                name,
                tags,
                v4,
                v6,
            },
            OpBody::RenameDevice { device, name } => Self::RenameDevice { device, name },
            OpBody::AssignDevice { device, account } => Self::AssignDevice { device, account },
            OpBody::Genesis { .. }
            | OpBody::AddAdmin { .. }
            | OpBody::RemoveAdmin { .. }
            | OpBody::SetThreshold { .. }
            | OpBody::Join(_)
            | OpBody::AddAccount { .. }
            | OpBody::RemoveAccount { .. }
            | OpBody::SetRole { .. }
            | OpBody::AuthorizeConsole { .. }
            | OpBody::RevokeConsole { .. }
            | OpBody::SetRecoveryKey { .. }
            | OpBody::Approved { .. }
            | OpBody::Acted { .. } => return None,
        })
    }
}
