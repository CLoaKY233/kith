use core::{fmt, str::FromStr};

use crate::Error;

/// A network interface name: `[A-Za-z0-9._-]`, 1 to 15 bytes, not starting with `-` or `.`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Interface(String);

impl Interface {
    /// Validates `name`.
    pub fn new(name: &str) -> Result<Self, Error> {
        let ok = (1..=15).contains(&name.len())
            && !name.starts_with(['-', '.'])
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if ok {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::InvalidInterface(name.to_owned()))
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Interface {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Self::new(s)
    }
}

impl fmt::Display for Interface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A lowercase DNS domain of letter-digit-hyphen labels, such as `kith` or `corp.example`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Domain(String);

impl Domain {
    /// Validates `name`, lowercasing it.
    pub fn new(name: &str) -> Result<Self, Error> {
        let lower = name.to_ascii_lowercase();
        let label_ok = |l: &str| {
            (1..=63).contains(&l.len())
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        if lower.len() <= 253 && lower.split('.').all(label_ok) {
            Ok(Self(lower))
        } else {
            Err(Error::InvalidDomain(name.to_owned()))
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Domain {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Self::new(s)
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `name` is a macOS network service name kith will pass to `networksetup`: like
/// [`valid_arg`], plus spaces and parentheses, and never starting with `-`.
pub(crate) fn valid_service(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b' ' | b'.' | b'_' | b'-' | b'/' | b'(' | b')' | b'+')
        })
}

/// Whether `arg` uses only characters that are inert to every tool kith invokes.
pub(crate) fn valid_arg(arg: &str) -> bool {
    !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'.' | b'_' | b'-' | b':' | b'/' | b'~' | b'=' | b'%')
        })
}
