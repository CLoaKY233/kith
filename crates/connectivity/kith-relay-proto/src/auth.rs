//! Challenge-response authentication, bound to the relay's identity against replay.

use alloc::vec::Vec;

use kith_crypto::{Context, Signature, SignatureError, SigningKey, VerifyingKey};
use kith_types::NodeId;

use crate::frame::NONCE_LEN;

/// Domain separation for relay challenge signatures.
pub const AUTH_CONTEXT: Context = Context::new("kith 2026-10 relay auth v1");

fn message(nonce: &[u8; NONCE_LEN], relay: &NodeId) -> Vec<u8> {
    let mut message = Vec::with_capacity(NONCE_LEN.saturating_add(relay.as_bytes().len()));
    message.extend_from_slice(nonce);
    message.extend_from_slice(relay.as_bytes());
    message
}

/// Signs the answer to `relay`'s challenge `nonce`.
#[must_use]
pub fn sign_challenge(key: &SigningKey, nonce: &[u8; NONCE_LEN], relay: &NodeId) -> Signature {
    key.sign(AUTH_CONTEXT, &message(nonce, relay))
}

/// Checks that `signature` is `node`'s answer to `relay`'s challenge `nonce`.
pub fn verify_challenge(
    node: NodeId,
    nonce: &[u8; NONCE_LEN],
    relay: &NodeId,
    signature: &Signature,
) -> Result<(), SignatureError> {
    VerifyingKey::from_node_id(node)?.verify(AUTH_CONTEXT, &message(nonce, relay), signature)
}
