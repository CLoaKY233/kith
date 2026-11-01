//! Join proofs: how a device redeems an invite.
//!
//! The invite key signs `(network, device, name)`, proving the joiner holds the invite. The
//! device key signs `(network, invite key)`, so nobody can enrol someone else's identity.
//! Messages are fixed-width with the name last, so they need no framing:
//!
//! ```text
//! invite message = network (32) | device (32) | name (UTF-8)
//! device message = network (32) | invite key (32)
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use kith_crypto::{Context, Signature, SigningKey, VerifyingKey};
use kith_types::{NetworkId, NodeId, Timestamp};
use serde::{Deserialize, Serialize};

use crate::LedgerError;
use crate::op::check_name;

/// Signs the invite message.
pub(crate) const INVITE_REDEEM_CONTEXT: Context =
    Context::new("kith 2026-10 ledger invite redeem v1");

/// Signs the device message.
pub(crate) const DEVICE_CONSENT_CONTEXT: Context =
    Context::new("kith 2026-10 ledger device consent v1");

/// Evidence that a device redeemed an invite. Carried by [`OpBody::Join`](crate::OpBody::Join).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinProof {
    /// The joining device.
    pub device: NodeId,
    /// The joining device's name, at most [`MAX_NAME_LEN`](crate::MAX_NAME_LEN) bytes.
    pub name: String,
    /// The invite being redeemed.
    pub invite_key: VerifyingKey,
    /// The invite key's signature over the invite message.
    pub invite_signature: Signature,
    /// The device key's signature over the device message.
    pub device_signature: Signature,
}

impl JoinProof {
    /// Creates a proof that `device_key` redeems `invite_key` in `network` under `name`.
    pub fn create(
        network: NetworkId,
        invite_key: &SigningKey,
        device_key: &SigningKey,
        name: &str,
    ) -> Result<Self, LedgerError> {
        check_name(name)?;
        let device = device_key.node_id();
        let invite_public = invite_key.verifying_key();
        Ok(Self {
            device,
            name: String::from(name),
            invite_key: invite_public,
            invite_signature: invite_key.sign(
                INVITE_REDEEM_CONTEXT,
                &invite_message(network, device, name),
            ),
            device_signature: device_key.sign(
                DEVICE_CONSENT_CONTEXT,
                &device_message(network, &invite_public),
            ),
        })
    }

    /// Checks the name limit and both signatures for `network`.
    pub fn verify(&self, network: NetworkId) -> Result<(), LedgerError> {
        check_name(&self.name)?;
        let device_key = VerifyingKey::from_node_id(self.device)
            .map_err(|_| LedgerError::InvalidKey(self.device))?;
        self.invite_key
            .verify(
                INVITE_REDEEM_CONTEXT,
                &invite_message(network, self.device, &self.name),
                &self.invite_signature,
            )
            .map_err(|_| LedgerError::BadJoinProof)?;
        device_key
            .verify(
                DEVICE_CONSENT_CONTEXT,
                &device_message(network, &self.invite_key),
                &self.device_signature,
            )
            .map_err(|_| LedgerError::BadJoinProof)
    }
}

fn invite_message(network: NetworkId, device: NodeId, name: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(64 + name.len());
    message.extend_from_slice(network.as_bytes());
    message.extend_from_slice(device.as_bytes());
    message.extend_from_slice(name.as_bytes());
    message
}

fn device_message(network: NetworkId, invite_key: &VerifyingKey) -> Vec<u8> {
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(network.as_bytes());
    message.extend_from_slice(&invite_key.to_bytes());
    message
}

/// Signs an [`AdmissionRequest`].
pub(crate) const ADMISSION_REQUEST_CONTEXT: Context =
    Context::new("kith 2026-10 ledger admission request v1");

/// A device asking an admin to add it, with no invite. It grants nothing by itself: an admin
/// who approves it signs an ordinary `AddDevice` op. The device's signature over
/// `network (32) | device (32) | requested_at (8, big-endian) | name` stops anyone from
/// asking on behalf of a key they do not hold, and dates the request so it can expire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRequest {
    /// The network asked to join.
    pub network: NetworkId,
    /// The asking device.
    pub device: NodeId,
    /// The name it asks for, at most [`MAX_NAME_LEN`](crate::MAX_NAME_LEN) bytes.
    pub name: String,
    /// When the device made the request, by its own clock.
    pub requested_at: Timestamp,
    /// The device key's signature over the request.
    pub signature: Signature,
}

impl AdmissionRequest {
    /// Creates a request by `device_key` to join `network` under `name`, made at `now`.
    pub fn create(
        network: NetworkId,
        device_key: &SigningKey,
        name: &str,
        now: Timestamp,
    ) -> Result<Self, LedgerError> {
        check_name(name)?;
        let device = device_key.node_id();
        Ok(Self {
            network,
            device,
            name: String::from(name),
            requested_at: now,
            signature: device_key.sign(
                ADMISSION_REQUEST_CONTEXT,
                &admission_message(network, device, now, name),
            ),
        })
    }

    /// Checks the name limit and the device's signature.
    pub fn verify(&self) -> Result<(), LedgerError> {
        check_name(&self.name)?;
        let device_key = VerifyingKey::from_node_id(self.device)
            .map_err(|_| LedgerError::InvalidKey(self.device))?;
        device_key
            .verify(
                ADMISSION_REQUEST_CONTEXT,
                &admission_message(self.network, self.device, self.requested_at, &self.name),
                &self.signature,
            )
            .map_err(|_| LedgerError::BadJoinProof)
    }
}

fn admission_message(network: NetworkId, device: NodeId, at: Timestamp, name: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(72 + name.len());
    message.extend_from_slice(network.as_bytes());
    message.extend_from_slice(device.as_bytes());
    message.extend_from_slice(&at.as_millis().to_be_bytes());
    message.extend_from_slice(name.as_bytes());
    message
}
