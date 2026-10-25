//! The one registry of envelope message types.
//!
//! Every [`MessageType`] is declared here so protocols never collide. Blocks of 16 per
//! plane; a released value is never reused.

use crate::MessageType;

// 0x00..=0x0f: reserved.

// 0x10..=0x1f: trust plane, ledger.
/// An unsigned ledger operation (the bytes that are hashed and signed).
pub const LEDGER_UNSIGNED_OP: MessageType = MessageType(0x10);
/// A signed ledger operation (the wire and storage form).
pub const LEDGER_SIGNED_OP: MessageType = MessageType(0x11);

// 0x20..=0x2f: trust plane, identity.
/// A device's secret key file.
pub const IDENTITY_SECRET_KEY: MessageType = MessageType(0x20);
/// An invite token, as carried in a `kith://join/` link.
pub const IDENTITY_INVITE: MessageType = MessageType(0x21);

// 0x30..=0x3f: trust plane, policy.
/// An access-control policy document.
pub const POLICY_DOCUMENT: MessageType = MessageType(0x30);

// 0x40..=0x4f: connectivity plane, relay.
/// A relay protocol frame payload.
pub const RELAY_FRAME: MessageType = MessageType(0x40);

// 0x50..=0x5f: connectivity plane, discovery.
/// A sealed peer-discovery message.
pub const DISCO_MESSAGE: MessageType = MessageType(0x50);

// 0x60..=0x6f: control plane, ledger sync.
/// A ledger reconciliation message.
pub const SYNC_MESSAGE: MessageType = MessageType(0x60);
/// A control-channel message between members (join, sync, endpoint gossip).
pub const CHANNEL_MESSAGE: MessageType = MessageType(0x61);
/// A typed ledger record value (relay, DNS, exit advertisement).
pub const CONTROL_RECORD: MessageType = MessageType(0x62);

// 0x70..=0x7f: management plane, IPC.
/// A request or response between the `kith` command line and the daemon.
pub const IPC_MESSAGE: MessageType = MessageType(0x70);

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;

    use super::{
        CHANNEL_MESSAGE, CONTROL_RECORD, DISCO_MESSAGE, IDENTITY_INVITE, IDENTITY_SECRET_KEY,
        IPC_MESSAGE, LEDGER_SIGNED_OP, LEDGER_UNSIGNED_OP, POLICY_DOCUMENT, RELAY_FRAME,
        SYNC_MESSAGE,
    };

    #[test]
    fn values_are_unique() {
        let all = [
            LEDGER_UNSIGNED_OP,
            LEDGER_SIGNED_OP,
            IDENTITY_SECRET_KEY,
            IDENTITY_INVITE,
            POLICY_DOCUMENT,
            RELAY_FRAME,
            DISCO_MESSAGE,
            SYNC_MESSAGE,
            CHANNEL_MESSAGE,
            CONTROL_RECORD,
            IPC_MESSAGE,
        ];
        let unique: BTreeSet<u8> = all.iter().map(|ty| ty.0).collect();
        assert_eq!(unique.len(), all.len());
    }
}
