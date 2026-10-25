//! IP prefixes (CIDR ranges).
//!
//! A prefix is always canonical: host bits are zero. `10.1.2.3/8` is rejected, not silently
//! rewritten, because a typo in a route or ACL should be an error, not a different rule.

use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::str::FromStr;

use serde::{Deserialize, Serialize};

/// Why a prefix was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrefixError {
    /// The prefix length exceeds the address width.
    #[error("prefix length {len} exceeds {max}")]
    LengthTooLong {
        /// The length given.
        len: u8,
        /// The address width in bits.
        max: u8,
    },
    /// Bits beyond the prefix length are set, e.g. `10.1.2.3/8`.
    #[error("address has bits set beyond the prefix length")]
    HostBitsSet,
    /// The text was not `address/length`.
    #[error("expected `address/length`")]
    Syntax,
}

macro_rules! define_prefix {
    (
        $(#[$meta:meta])*
        $name:ident, $raw:ident = $raw_str:literal, $addr:ty, $bits:ty, $width:literal
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = $raw_str, into = $raw_str)]
        pub struct $name {
            addr: $addr,
            len: u8,
        }

        impl $name {
            /// Address width in bits.
            pub const WIDTH: u8 = $width;

            /// A canonical prefix. Fails if `len` is too long or host bits are set.
            pub fn new(addr: $addr, len: u8) -> Result<Self, PrefixError> {
                let mask = Self::mask_for(len)?;
                if <$bits>::from(addr) & !mask != 0 {
                    return Err(PrefixError::HostBitsSet);
                }
                Ok(Self { addr, len })
            }

            /// The prefix containing `addr`, clearing any host bits.
            pub fn truncating(addr: $addr, len: u8) -> Result<Self, PrefixError> {
                let mask = Self::mask_for(len)?;
                Ok(Self { addr: <$addr>::from(<$bits>::from(addr) & mask), len })
            }

            /// The network address (host bits zero).
            #[must_use]
            pub const fn addr(&self) -> $addr {
                self.addr
            }

            /// The prefix length in bits.
            #[must_use]
            pub const fn prefix_len(&self) -> u8 {
                self.len
            }

            /// Whether this prefix covers `addr`.
            #[must_use]
            pub fn contains(&self, addr: $addr) -> bool {
                // `len` was validated at construction, so the mask always exists.
                Self::mask_for(self.len)
                    .is_ok_and(|mask| <$bits>::from(addr) & mask == <$bits>::from(self.addr))
            }

            /// Whether this prefix covers every address in `other`.
            #[must_use]
            pub fn covers(&self, other: &Self) -> bool {
                self.len <= other.len && self.contains(other.addr)
            }

            fn mask_for(len: u8) -> Result<$bits, PrefixError> {
                let host_bits = Self::WIDTH
                    .checked_sub(len)
                    .ok_or(PrefixError::LengthTooLong { len, max: Self::WIDTH })?;
                // A zero-length prefix shifts by the full width, which `checked_shl` refuses:
                // that case is the all-zero mask.
                Ok(<$bits>::MAX.checked_shl(u32::from(host_bits)).unwrap_or(0))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}/{}", self.addr, self.len)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = PrefixError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let (addr, len) = text.split_once('/').ok_or(PrefixError::Syntax)?;
                let addr = addr.parse::<$addr>().map_err(|_| PrefixError::Syntax)?;
                let len = len.parse::<u8>().map_err(|_| PrefixError::Syntax)?;
                Self::new(addr, len)
            }
        }

        /// Unvalidated wire form; converting it back into the prefix re-validates.
        #[derive(Serialize, Deserialize)]
        struct $raw {
            addr: $addr,
            len: u8,
        }

        impl TryFrom<$raw> for $name {
            type Error = PrefixError;

            fn try_from(raw: $raw) -> Result<Self, Self::Error> {
                Self::new(raw.addr, raw.len)
            }
        }

        impl From<$name> for $raw {
            fn from(prefix: $name) -> Self {
                Self { addr: prefix.addr, len: prefix.len }
            }
        }
    };
}

define_prefix!(
    /// An IPv4 prefix such as `10.0.0.0/8`.
    Ipv4Prefix, RawIpv4Prefix = "RawIpv4Prefix", Ipv4Addr, u32, 32
);

define_prefix!(
    /// An IPv6 prefix such as `fd7a::/48`.
    Ipv6Prefix, RawIpv6Prefix = "RawIpv6Prefix", Ipv6Addr, u128, 128
);

/// An IPv4 or IPv6 prefix.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IpPrefix {
    /// IPv4.
    V4(Ipv4Prefix),
    /// IPv6.
    V6(Ipv6Prefix),
}

impl IpPrefix {
    /// Whether this prefix covers `addr`. Addresses of the other family never match.
    #[must_use]
    pub fn contains(&self, addr: IpAddr) -> bool {
        match (self, addr) {
            (Self::V4(prefix), IpAddr::V4(addr)) => prefix.contains(addr),
            (Self::V6(prefix), IpAddr::V6(addr)) => prefix.contains(addr),
            (Self::V4(_), IpAddr::V6(_)) | (Self::V6(_), IpAddr::V4(_)) => false,
        }
    }

    /// The prefix length in bits.
    #[must_use]
    pub const fn prefix_len(&self) -> u8 {
        match self {
            Self::V4(prefix) => prefix.prefix_len(),
            Self::V6(prefix) => prefix.prefix_len(),
        }
    }
}

impl From<Ipv4Prefix> for IpPrefix {
    fn from(prefix: Ipv4Prefix) -> Self {
        Self::V4(prefix)
    }
}

impl From<Ipv6Prefix> for IpPrefix {
    fn from(prefix: Ipv6Prefix) -> Self {
        Self::V6(prefix)
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(prefix) => fmt::Display::fmt(prefix, f),
            Self::V6(prefix) => fmt::Display::fmt(prefix, f),
        }
    }
}

impl fmt::Debug for IpPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for IpPrefix {
    type Err = PrefixError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.contains(':') {
            text.parse().map(Self::V6)
        } else {
            text.parse().map(Self::V4)
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use alloc::string::ToString;
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use proptest::prelude::*;

    use super::{IpPrefix, Ipv4Prefix, Ipv6Prefix, PrefixError};

    #[test]
    fn parses_and_displays() {
        let prefix: IpPrefix = "10.0.0.0/8".parse().unwrap();
        assert_eq!(prefix.to_string(), "10.0.0.0/8");
        let prefix: IpPrefix = "fd7a:115c::/32".parse().unwrap();
        assert_eq!(prefix.to_string(), "fd7a:115c::/32");
    }

    #[test]
    fn rejects_host_bits_and_long_lengths() {
        assert_eq!(
            "10.1.2.3/8".parse::<Ipv4Prefix>(),
            Err(PrefixError::HostBitsSet)
        );
        assert_eq!(
            "10.0.0.0/33".parse::<Ipv4Prefix>(),
            Err(PrefixError::LengthTooLong { len: 33, max: 32 })
        );
        assert_eq!("10.0.0.0".parse::<Ipv4Prefix>(), Err(PrefixError::Syntax));
        assert_eq!(
            Ipv6Prefix::new(Ipv6Addr::LOCALHOST, 129),
            Err(PrefixError::LengthTooLong { len: 129, max: 128 })
        );
    }

    #[test]
    fn zero_length_covers_everything() {
        let all = Ipv4Prefix::new(Ipv4Addr::UNSPECIFIED, 0).unwrap();
        assert!(all.contains(Ipv4Addr::BROADCAST));
        let all = Ipv6Prefix::new(Ipv6Addr::UNSPECIFIED, 0).unwrap();
        assert!(all.contains(Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn families_never_cross() {
        let v4: IpPrefix = "0.0.0.0/0".parse().unwrap();
        assert!(!v4.contains(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn deserialization_revalidates() {
        // Hand-encode the raw form of `10.1.2.3/8`, which must be rejected.
        let bad = postcard::to_allocvec(&([10_u8, 1, 2, 3], 8_u8)).unwrap();
        assert!(postcard::from_bytes::<Ipv4Prefix>(&bad).is_err());
        let good: Ipv4Prefix = "10.0.0.0/8".parse().unwrap();
        let bytes = postcard::to_allocvec(&good).unwrap();
        assert_eq!(postcard::from_bytes::<Ipv4Prefix>(&bytes).unwrap(), good);
    }

    proptest! {
        #[test]
        fn truncating_always_contains_its_input(bits in any::<u32>(), len in 0_u8..=32) {
            let addr = Ipv4Addr::from(bits);
            let prefix = Ipv4Prefix::truncating(addr, len).unwrap();
            prop_assert!(prefix.contains(addr));
            prop_assert_eq!(Ipv4Prefix::new(prefix.addr(), len), Ok(prefix));
        }

        #[test]
        fn shorter_truncation_covers_longer(bits in any::<u128>(), a in 0_u8..=128, b in 0_u8..=128) {
            let (short, long) = if a <= b { (a, b) } else { (b, a) };
            let addr = Ipv6Addr::from(bits);
            let outer = Ipv6Prefix::truncating(addr, short).unwrap();
            let inner = Ipv6Prefix::truncating(addr, long).unwrap();
            prop_assert!(outer.covers(&inner));
        }

        #[test]
        fn text_round_trips(bits in any::<u32>(), len in 0_u8..=32) {
            let prefix = IpPrefix::V4(Ipv4Prefix::truncating(Ipv4Addr::from(bits), len).unwrap());
            prop_assert_eq!(prefix.to_string().parse::<IpPrefix>(), Ok(prefix));
        }
    }
}
