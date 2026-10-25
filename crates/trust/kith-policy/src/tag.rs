//! Tag names: the labels that group devices in policy rules.
//!
//! Tags are compared byte-for-byte, so the alphabet is deliberately tiny. A restricted,
//! lowercase-only form means `Server` and `server` can never be two different groups, and a
//! tag can be pasted into a shell or a DNS label without quoting.

use alloc::string::String;
use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};

/// The longest tag name, in bytes.
pub const MAX_TAG_LEN: usize = 32;

/// Why a tag name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TagError {
    /// The name was empty.
    #[error("tag name is empty")]
    Empty,
    /// The name was longer than [`MAX_TAG_LEN`] bytes.
    #[error("tag name is {len} bytes, longer than {MAX_TAG_LEN}")]
    TooLong {
        /// The length given, in bytes.
        len: usize,
    },
    /// The name started with `-`.
    #[error("tag name starts with '-'")]
    LeadingHyphen,
    /// The name contained a byte outside `[a-z0-9-]`.
    #[error("tag name contains a byte outside [a-z0-9-] at offset {offset}")]
    InvalidByte {
        /// The offset of the first offending byte.
        offset: usize,
    },
}

/// A validated tag name such as `server` or `team-ops`.
///
/// Invariant: 1 to [`MAX_TAG_LEN`] bytes of `[a-z0-9-]`, not starting with `-`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct TagName(String);

impl TagName {
    /// Validates and wraps `name`.
    pub fn new(name: &str) -> Result<Self, TagError> {
        validate(name)?;
        Ok(Self(String::from(name)))
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate(name: &str) -> Result<(), TagError> {
    let bytes = name.as_bytes();
    // Length first, so an oversized input is rejected before it is scanned.
    if bytes.len() > MAX_TAG_LEN {
        return Err(TagError::TooLong { len: bytes.len() });
    }
    match bytes.first() {
        None => return Err(TagError::Empty),
        Some(b'-') => return Err(TagError::LeadingHyphen),
        Some(_) => {}
    }
    let invalid = bytes
        .iter()
        .position(|&byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    match invalid {
        Some(offset) => Err(TagError::InvalidByte { offset }),
        None => Ok(()),
    }
}

impl TryFrom<String> for TagName {
    type Error = TagError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        validate(&name)?;
        Ok(Self(name))
    }
}

impl FromStr for TagName {
    type Err = TagError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::new(name)
    }
}

impl fmt::Display for TagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TagName({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TAG_LEN, TagError, TagName};

    #[test]
    fn accepts_valid_names() {
        for name in [
            "a",
            "server",
            "team-ops",
            "0",
            "a-",
            "x".repeat(MAX_TAG_LEN).as_str(),
        ] {
            assert_eq!(TagName::new(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn rejects_invalid_names() {
        assert_eq!(TagName::new(""), Err(TagError::Empty));
        assert_eq!(TagName::new("-a"), Err(TagError::LeadingHyphen));
        assert_eq!(
            TagName::new("Server"),
            Err(TagError::InvalidByte { offset: 0 })
        );
        assert_eq!(
            TagName::new("a_b"),
            Err(TagError::InvalidByte { offset: 1 })
        );
        assert_eq!(TagName::new("é"), Err(TagError::InvalidByte { offset: 0 }));
        assert_eq!(
            TagName::new(&"x".repeat(MAX_TAG_LEN + 1)),
            Err(TagError::TooLong {
                len: MAX_TAG_LEN + 1
            })
        );
    }
}
