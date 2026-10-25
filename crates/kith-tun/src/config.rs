//! Device configuration, validated before any syscall.

use std::net::{Ipv4Addr, Ipv6Addr};

/// Smallest accepted MTU (the IPv4 minimum).
pub const MIN_MTU: u16 = 576;

/// Smallest accepted MTU with IPv6.
pub const MIN_MTU_IPV6: u16 = 1280;

/// Largest accepted MTU.
pub const MAX_MTU: u16 = 9000;

/// Longest interface name (`IFNAMSIZ` minus the NUL).
pub const MAX_NAME_LEN: usize = 15;

/// Settings for a new TUN device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunConfig {
    /// Interface name, or `None` for an OS-chosen one. Must be `utunN` on macOS.
    pub name: Option<String>,
    /// Interface MTU.
    pub mtu: u16,
    /// IPv4 address and prefix length (0..=32) assigned to the interface.
    pub ipv4: Option<(Ipv4Addr, u8)>,
    /// IPv6 address and prefix length (0..=128) assigned to the interface.
    pub ipv6: Option<(Ipv6Addr, u8)>,
    /// Request GSO/GRO offload; Linux only, falls back silently.
    pub offload: bool,
}

/// Why a [`TunConfig`] was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The name is empty.
    #[error("interface name is empty")]
    NameEmpty,
    /// The name is longer than [`MAX_NAME_LEN`] bytes.
    #[error("interface name is {len} bytes, the limit is {MAX_NAME_LEN}")]
    NameTooLong {
        /// Length of the rejected name in bytes.
        len: usize,
    },
    /// The name contains a byte other than ASCII letters, digits, `-`, `_` or `.`.
    #[error("interface name may only contain ASCII letters, digits, '-', '_' and '.'")]
    NameInvalidCharacter,
    /// macOS only creates `utunN` devices.
    #[error("on macOS the interface name must be `utun` followed by a number")]
    NameNotUtun,
    /// The MTU is outside [`MIN_MTU`]..=[`MAX_MTU`].
    #[error("MTU {mtu} is outside {MIN_MTU}..={MAX_MTU}")]
    MtuOutOfRange {
        /// The rejected MTU.
        mtu: u16,
    },
    /// IPv6 is configured but the MTU is below [`MIN_MTU_IPV6`].
    #[error("MTU {mtu} is below the IPv6 minimum of {MIN_MTU_IPV6}")]
    MtuTooSmallForIpv6 {
        /// The rejected MTU.
        mtu: u16,
    },
    /// The IPv4 prefix length is above 32.
    #[error("IPv4 prefix length {prefix} is above 32")]
    Ipv4PrefixOutOfRange {
        /// The rejected prefix length.
        prefix: u8,
    },
    /// The IPv6 prefix length is above 128.
    #[error("IPv6 prefix length {prefix} is above 128")]
    Ipv6PrefixOutOfRange {
        /// The rejected prefix length.
        prefix: u8,
    },
}

impl TunConfig {
    /// Checks every field, returning the first problem found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(name) = &self.name {
            validate_name(name)?;
        }
        if !(MIN_MTU..=MAX_MTU).contains(&self.mtu) {
            return Err(ConfigError::MtuOutOfRange { mtu: self.mtu });
        }
        if let Some((_, prefix)) = self.ipv4
            && prefix > 32
        {
            return Err(ConfigError::Ipv4PrefixOutOfRange { prefix });
        }
        if let Some((_, prefix)) = self.ipv6 {
            if prefix > 128 {
                return Err(ConfigError::Ipv6PrefixOutOfRange { prefix });
            }
            if self.mtu < MIN_MTU_IPV6 {
                return Err(ConfigError::MtuTooSmallForIpv6 { mtu: self.mtu });
            }
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() {
        return Err(ConfigError::NameEmpty);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(ConfigError::NameTooLong { len: name.len() });
    }
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.');
    if !name.bytes().all(allowed) {
        return Err(ConfigError::NameInvalidCharacter);
    }
    if cfg!(target_os = "macos") && !is_utun_name(name) {
        return Err(ConfigError::NameNotUtun);
    }
    Ok(())
}

fn is_utun_name(name: &str) -> bool {
    name.strip_prefix("utun")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_good_configs_and_names_the_first_problem() {
        let base = TunConfig {
            name: None,
            mtu: 1420,
            ipv4: Some((Ipv4Addr::new(100, 64, 0, 1), 10)),
            ipv6: Some((Ipv6Addr::LOCALHOST, 64)),
            offload: true,
        };
        let portable_name = if cfg!(target_os = "macos") {
            "utun9"
        } else {
            "kith0"
        };
        let with = |edit: &dyn Fn(&mut TunConfig)| {
            let mut config = base.clone();
            edit(&mut config);
            config
        };
        let cases: Vec<(&str, TunConfig, Result<(), ConfigError>)> = vec![
            ("defaults", base.clone(), Ok(())),
            (
                "valid name",
                with(&|c| c.name = Some(portable_name.into())),
                Ok(()),
            ),
            (
                "ipv4 only at minimum mtu",
                with(&|c| {
                    c.ipv6 = None;
                    c.mtu = MIN_MTU;
                }),
                Ok(()),
            ),
            (
                "prefix bounds inclusive",
                with(&|c| {
                    c.ipv4 = Some((Ipv4Addr::UNSPECIFIED, 32));
                    c.ipv6 = Some((Ipv6Addr::UNSPECIFIED, 128));
                }),
                Ok(()),
            ),
            (
                "empty name",
                with(&|c| c.name = Some(String::new())),
                Err(ConfigError::NameEmpty),
            ),
            (
                "long name",
                with(&|c| c.name = Some("utun1234567890123".into())),
                Err(ConfigError::NameTooLong { len: 17 }),
            ),
            (
                "slash in name",
                with(&|c| c.name = Some("ab/c".into())),
                Err(ConfigError::NameInvalidCharacter),
            ),
            (
                "mtu too small",
                with(&|c| c.mtu = MIN_MTU - 1),
                Err(ConfigError::MtuOutOfRange { mtu: MIN_MTU - 1 }),
            ),
            (
                "mtu too large",
                with(&|c| c.mtu = MAX_MTU + 1),
                Err(ConfigError::MtuOutOfRange { mtu: MAX_MTU + 1 }),
            ),
            (
                "ipv6 needs 1280",
                with(&|c| c.mtu = MIN_MTU_IPV6 - 1),
                Err(ConfigError::MtuTooSmallForIpv6 {
                    mtu: MIN_MTU_IPV6 - 1,
                }),
            ),
            (
                "ipv4 prefix",
                with(&|c| c.ipv4 = Some((Ipv4Addr::UNSPECIFIED, 33))),
                Err(ConfigError::Ipv4PrefixOutOfRange { prefix: 33 }),
            ),
            (
                "ipv6 prefix",
                with(&|c| c.ipv6 = Some((Ipv6Addr::UNSPECIFIED, 129))),
                Err(ConfigError::Ipv6PrefixOutOfRange { prefix: 129 }),
            ),
        ];
        for (label, config, expected) in cases {
            assert_eq!(config.validate(), expected, "case: {label}");
        }
        let utun_rule = with(&|c| c.name = Some("kith0".into())).validate();
        if cfg!(target_os = "macos") {
            assert_eq!(utun_rule, Err(ConfigError::NameNotUtun));
            assert_eq!(
                with(&|c| c.name = Some("utun".into())).validate(),
                Err(ConfigError::NameNotUtun)
            );
        } else {
            assert_eq!(utun_rule, Ok(()));
        }
    }
}
