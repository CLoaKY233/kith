//! Envelope message types owned by this crate, as assigned in [`kith_wire::registry`].

use kith_wire::{MessageType, registry};

/// A device's secret-key file ([`crate::DeviceIdentity::to_secret_bytes`]).
pub(crate) const SECRET_KEY: MessageType = registry::IDENTITY_SECRET_KEY;

/// An invite token ([`crate::InviteToken`]), before base32 and checksum.
pub(crate) const INVITE: MessageType = registry::IDENTITY_INVITE;
