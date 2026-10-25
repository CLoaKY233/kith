//! 32-byte identifiers.
//!
//! - [`NodeId`]: a device's long-term Ed25519 public key. The key *is* the identity.
//! - [`NetworkId`]: the hash of a network's genesis operation.
//! - [`OpId`]: the content hash of one ledger operation.
//!
//! They are distinct types so they can never be confused for one another.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::hex;

/// Number of bytes in every identifier.
const LEN: usize = 32;

/// Number of leading bytes shown by [`ShortId`] and `Debug`.
const SHORT_LEN: usize = 4;

/// An identifier string was not exactly 64 hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("expected 64 hexadecimal digits")]
pub struct ParseIdError;

/// Displays the first four bytes of an identifier, for logs and humans.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShortId([u8; SHORT_LEN]);

impl ShortId {
    fn of(bytes: &[u8; LEN]) -> Self {
        let mut short = [0_u8; SHORT_LEN];
        for (slot, byte) in short.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        Self(short)
    }
}

impl fmt::Display for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex::write(f, &self.0)
    }
}

impl fmt::Debug for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name([u8; LEN]);

        impl $name {
            /// Wraps raw bytes.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; LEN]) -> Self {
                Self(bytes)
            }

            /// Borrows the raw bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; LEN] {
                &self.0
            }

            /// Copies out the raw bytes.
            #[must_use]
            pub const fn to_bytes(self) -> [u8; LEN] {
                self.0
            }

            /// The first four bytes, for compact display.
            #[must_use]
            pub fn short(&self) -> ShortId {
                ShortId::of(&self.0)
            }
        }

        impl From<[u8; LEN]> for $name {
            fn from(bytes: [u8; LEN]) -> Self {
                Self(bytes)
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        /// Full lowercase hex: 64 digits.
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                hex::write(f, &self.0)
            }
        }

        /// Short form, so logs stay readable: `NodeId(1a2b3c4d)`.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.short())
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                hex::decode::<LEN>(text).map(Self).ok_or(ParseIdError)
            }
        }
    };
}

define_id!(
    /// A device's identity: its long-term Ed25519 public key.
    NodeId
);

define_id!(
    /// A network's identity: the hash of its genesis operation.
    NetworkId
);

define_id!(
    /// The content hash of one signed ledger operation.
    OpId
);

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use alloc::format;
    use alloc::string::ToString;

    use proptest::prelude::*;

    use super::{NodeId, OpId, ParseIdError};

    #[test]
    fn display_is_full_hex_and_debug_is_short() {
        let mut bytes = [0_u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0x01;
        let id = NodeId::from_bytes(bytes);
        let text = id.to_string();
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab000000"));
        assert!(text.ends_with("01"));
        assert_eq!(format!("{id:?}"), "NodeId(ab000000)");
    }

    #[test]
    fn rejects_malformed_text() {
        assert_eq!("".parse::<OpId>(), Err(ParseIdError));
        assert_eq!("zz".repeat(32).parse::<OpId>(), Err(ParseIdError));
        assert_eq!("00".repeat(33).parse::<OpId>(), Err(ParseIdError));
    }

    #[test]
    fn serializes_as_32_raw_bytes() {
        let id = NodeId::from_bytes([7; 32]);
        let encoded = postcard::to_allocvec(&id).unwrap();
        assert_eq!(encoded.len(), 32);
        assert_eq!(postcard::from_bytes::<NodeId>(&encoded).unwrap(), id);
    }

    proptest! {
        #[test]
        fn text_round_trips(bytes in any::<[u8; 32]>()) {
            let id = NodeId::from_bytes(bytes);
            prop_assert_eq!(id.to_string().parse::<NodeId>(), Ok(id));
            prop_assert_eq!(id.to_string().to_uppercase().parse::<NodeId>(), Ok(id));
        }

        #[test]
        fn ordering_matches_bytes(a in any::<[u8; 32]>(), b in any::<[u8; 32]>()) {
            prop_assert_eq!(NodeId::from_bytes(a).cmp(&NodeId::from_bytes(b)), a.cmp(&b));
        }
    }
}
