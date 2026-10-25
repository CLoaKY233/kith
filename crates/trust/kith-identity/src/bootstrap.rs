//! Bootstrap endpoints carried in an invite: where a joining device first knocks.

use alloc::string::String;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};
use core::str::FromStr;

use serde::{Deserialize, Serialize};

/// The longest hostname accepted, in bytes (RFC 1035 presentation form, no trailing dot).
pub const MAX_HOSTNAME_LEN: usize = 253;

/// The longest single DNS label, in bytes.
const MAX_LABEL_LEN: usize = 63;

/// Why a bootstrap endpoint was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BootstrapError {
    /// The hostname is empty or longer than [`MAX_HOSTNAME_LEN`].
    #[error("hostname length must be 1..={MAX_HOSTNAME_LEN} bytes")]
    HostnameLength,
    /// A label is empty, too long, has a bad character or starts/ends with a hyphen.
    #[error("invalid hostname label")]
    InvalidLabel,
    /// The last label is all digits, so the name would be confused with an IPv4 address.
    #[error("hostname has a numeric top-level label")]
    NumericTopLevel,
    /// An IPv6 socket address carried a flow label or scope id, which mean nothing to
    /// another device.
    #[error("IPv6 bootstrap address must not carry a flow label or scope id")]
    ScopedAddress,
}

/// A validated DNS hostname: LDH labels of 1–63 bytes, at most [`MAX_HOSTNAME_LEN`] bytes,
/// stored in lowercase so that equal names compare and encode identically.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Hostname(String);

impl Hostname {
    /// Validates and lowercases `name`. A single trailing dot is not accepted.
    pub fn new(name: &str) -> Result<Self, BootstrapError> {
        if name.is_empty() || name.len() > MAX_HOSTNAME_LEN {
            return Err(BootstrapError::HostnameLength);
        }
        let mut last = "";
        for label in name.split('.') {
            let bytes = label.as_bytes();
            let valid_length = (1..=MAX_LABEL_LEN).contains(&bytes.len());
            let valid_chars = bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-');
            let hyphen_edge = bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-');
            if !valid_length || !valid_chars || hyphen_edge {
                return Err(BootstrapError::InvalidLabel);
            }
            last = label;
        }
        if last.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(BootstrapError::NumericTopLevel);
        }
        Ok(Self(name.to_ascii_lowercase()))
    }

    /// The hostname in lowercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Hostname {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let name = Self::new(&raw).map_err(serde::de::Error::custom)?;
        if name.0 == raw {
            Ok(name)
        } else {
            // Only the canonical lowercase form has a valid encoding.
            Err(serde::de::Error::custom("hostname is not lowercase"))
        }
    }
}

impl FromStr for Hostname {
    type Err = BootstrapError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::new(name)
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hostname({:?})", self.0)
    }
}

/// An endpoint a joining device contacts first: a member or relay reachable from outside.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bootstrap {
    /// A literal IPv4 or IPv6 socket address. IPv6 addresses carry no flow label or scope.
    Socket(SocketAddr),
    /// A hostname resolved by the joining device, and a port.
    Host(Hostname, u16),
}

impl Bootstrap {
    /// Checks that an IPv6 address carries no flow label or scope id.
    pub fn validate(&self) -> Result<(), BootstrapError> {
        match self {
            Self::Socket(SocketAddr::V6(address))
                if address.flowinfo() != 0 || address.scope_id() != 0 =>
            {
                Err(BootstrapError::ScopedAddress)
            }
            Self::Socket(_) | Self::Host(..) => Ok(()),
        }
    }
}

impl fmt::Display for Bootstrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket(address) => write!(f, "{address}"),
            Self::Host(host, port) => write!(f, "{host}:{port}"),
        }
    }
}

/// The stable wire form. Explicit variants keep the encoding independent of serde's
/// representation of `core::net` types.
#[derive(Serialize, Deserialize)]
enum BootstrapWire {
    V4([u8; 4], u16),
    V6([u8; 16], u16),
    Host(Hostname, u16),
}

impl Serialize for Bootstrap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::Socket(SocketAddr::V4(address)) => {
                BootstrapWire::V4(address.ip().octets(), address.port())
            }
            Self::Socket(SocketAddr::V6(address)) => {
                BootstrapWire::V6(address.ip().octets(), address.port())
            }
            Self::Host(host, port) => BootstrapWire::Host(host.clone(), *port),
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bootstrap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match BootstrapWire::deserialize(deserializer)? {
            BootstrapWire::V4(octets, port) => {
                Self::Socket(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port))
            }
            BootstrapWire::V6(octets, port) => Self::Socket(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(octets),
                port,
                0,
                0,
            ))),
            BootstrapWire::Host(host, port) => Self::Host(host, port),
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use core::net::{SocketAddr, SocketAddrV6};

    use super::{Bootstrap, BootstrapError, Hostname, MAX_HOSTNAME_LEN};

    #[test]
    fn accepts_and_lowercases_hostnames() {
        let host = Hostname::new("Relay-1.Example.COM").unwrap();
        assert_eq!(host.as_str(), "relay-1.example.com");
        assert_eq!(host.to_string(), "relay-1.example.com");
        assert!(Hostname::new("localhost").is_ok());
        assert!(Hostname::new("1.example.c0m").is_ok());
    }

    #[test]
    fn rejects_bad_hostnames() {
        assert_eq!(Hostname::new(""), Err(BootstrapError::HostnameLength));
        let mut long = "a.".repeat(MAX_HOSTNAME_LEN / 2);
        long.push_str("ab");
        assert_eq!(Hostname::new(&long), Err(BootstrapError::HostnameLength));
        for bad in ["a..b", "-a.com", "a-.com", "a_b.com", "a.com.", "é.com"] {
            assert_eq!(
                Hostname::new(bad),
                Err(BootstrapError::InvalidLabel),
                "{bad}"
            );
        }
        assert_eq!(
            Hostname::new(&"a".repeat(64)),
            Err(BootstrapError::InvalidLabel)
        );
        assert_eq!(
            Hostname::new("10.0.0.1"),
            Err(BootstrapError::NumericTopLevel)
        );
    }

    #[test]
    fn rejects_scoped_ipv6() {
        let scoped = Bootstrap::Socket(SocketAddr::V6(SocketAddrV6::new(
            "fe80::1".parse().unwrap(),
            1,
            0,
            3,
        )));
        assert_eq!(scoped.validate(), Err(BootstrapError::ScopedAddress));
        let plain = Bootstrap::Socket("[2001:db8::1]:41641".parse().unwrap());
        assert_eq!(plain.validate(), Ok(()));
    }

    #[test]
    fn displays_endpoints() {
        let endpoints = [
            Bootstrap::Socket("192.0.2.1:41641".parse().unwrap()),
            Bootstrap::Host(Hostname::new("relay.example").unwrap(), 443),
        ];
        assert_eq!(endpoints[0].to_string(), "192.0.2.1:41641");
        assert_eq!(endpoints[1].to_string(), "relay.example:443");
    }
}
