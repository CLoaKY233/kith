//! Validated keys.
//!
//! The alphabet `[a-z0-9._-]` with no leading dot means a segment can never
//! be `.` or `..`, never contain a separator or NUL, and never be
//! case-folded into another key on case-insensitive filesystems. Mapping a
//! key to a path therefore cannot escape the store root.

use core::fmt;

use crate::KeyError;

/// Maximum length of a [`Name`] or of one [`Namespace`] segment, in bytes.
pub const MAX_SEGMENT_LEN: usize = 128;

/// Maximum total length of a [`Namespace`], separators included, in bytes.
pub const MAX_NAMESPACE_LEN: usize = 255;

fn validate_segment(segment: &str) -> Result<(), KeyError> {
    let bytes = segment.as_bytes();
    let Some(&first) = bytes.first() else {
        return Err(KeyError::Empty);
    };
    if bytes.len() > MAX_SEGMENT_LEN {
        return Err(KeyError::TooLong);
    }
    if let Some(&bad) = bytes
        .iter()
        .find(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        return Err(KeyError::InvalidByte(bad));
    }
    if first == b'.' {
        return Err(KeyError::LeadingDot);
    }
    Ok(())
}

/// A key name: 1..=128 bytes of `[a-z0-9._-]`, not starting with `.`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    /// Validates `name`.
    pub fn new(name: &str) -> Result<Self, KeyError> {
        validate_segment(name)?;
        Ok(Self(name.to_owned()))
    }

    /// The name as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A namespace: one or more `/`-separated segments, each valid as a
/// [`Name`], at most [`MAX_NAMESPACE_LEN`] bytes in total.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace(String);

impl Namespace {
    /// Validates `namespace`, e.g. `"settings"` or `"ledger/ab12"`.
    pub fn new(namespace: &str) -> Result<Self, KeyError> {
        if namespace.len() > MAX_NAMESPACE_LEN {
            return Err(KeyError::TooLong);
        }
        namespace.split('/').try_for_each(validate_segment)?;
        Ok(Self(namespace.to_owned()))
    }

    /// Returns the sub-namespace `self/segment`.
    pub fn join(&self, segment: &str) -> Result<Self, KeyError> {
        validate_segment(segment)?;
        Self::new(&format!("{}/{segment}", self.0))
    }

    /// The namespace as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The segments, outermost first. Each is a valid segment.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

/// A full key: a namespace and a name within it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// Where the value lives.
    pub namespace: Namespace,
    /// The value's name within the namespace.
    pub name: Name,
}

impl Key {
    /// Builds a key from already validated parts.
    pub const fn new(namespace: Namespace, name: Name) -> Self {
        Self { namespace, name }
    }

    /// Validates and builds a key from strings.
    pub fn parse(namespace: &str, name: &str) -> Result<Self, KeyError> {
        Ok(Self::new(Namespace::new(namespace)?, Name::new(name)?))
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.namespace, self.name)
    }
}

impl TryFrom<&str> for Name {
    type Error = KeyError;
    fn try_from(value: &str) -> Result<Self, KeyError> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Namespace {
    type Error = KeyError;
    fn try_from(value: &str) -> Result<Self, KeyError> {
        Self::new(value)
    }
}
