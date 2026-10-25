//! Validated, case-insensitive domain names.
//!
//! Every name in a zone, a record target or a split-DNS route is a [`DomainName`], so
//! the zone never holds a name that cannot be written to the wire. Names are stored
//! lowercased in dotted form without the trailing dot; that makes equality, ordering
//! and `BTreeMap` lookups case-insensitive for free, and lets the answer engine look a
//! query name up as a borrowed `&str` without allocating.

use alloc::string::String;
use core::borrow::Borrow;
use core::fmt;
use core::str::FromStr;

/// Longest permitted name in dotted form, without the trailing dot (RFC 1035 §2.3.4:
/// 255 bytes on the wire, which is 253 bytes of text).
pub const MAX_NAME_LEN: usize = 253;

/// Longest permitted label (RFC 1035 §2.3.4).
pub const MAX_LABEL_LEN: usize = 63;

/// Why a string is not a valid [`DomainName`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NameError {
    /// The name has no labels (the root name is never a valid zone name).
    #[error("domain name is empty")]
    Empty,
    /// The name is longer than [`MAX_NAME_LEN`] bytes.
    #[error("domain name is longer than {} bytes", MAX_NAME_LEN)]
    TooLong,
    /// Two dots in a row, or a leading dot.
    #[error("domain name has an empty label")]
    EmptyLabel,
    /// A label is longer than [`MAX_LABEL_LEN`] bytes.
    #[error("label is longer than {} bytes", MAX_LABEL_LEN)]
    LabelTooLong,
    /// A label holds a byte other than an ASCII letter, digit, `-` or `_`.
    #[error("label may hold only ASCII letters, digits, `-` and `_`")]
    InvalidByte,
    /// A label starts or ends with `-`.
    #[error("label starts or ends with `-`")]
    HyphenAtEdge,
    /// `*` appears somewhere other than as the whole leftmost label.
    #[error("`*` is allowed only as the whole leftmost label")]
    MisplacedWildcard,
}

/// A validated domain name.
///
/// Invariants:
/// - one or more labels, each 1 to [`MAX_LABEL_LEN`] bytes, the whole name at most
///   [`MAX_NAME_LEN`] bytes;
/// - labels hold only lowercase ASCII letters, digits, `-` (not at either edge) and `_`
///   (so SRV-style names such as `_http._tcp` work);
/// - `*` appears only as the entire leftmost label (an RFC 4592 wildcard owner).
///
/// Input is accepted in any case, with or without a trailing dot, and stored lowercased.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainName {
    text: String,
}

impl DomainName {
    /// Validates `input` and returns the lowercased name.
    pub fn new(input: &str) -> Result<Self, NameError> {
        let trimmed = input.strip_suffix('.').unwrap_or(input);
        if trimmed.is_empty() {
            return Err(NameError::Empty);
        }
        if trimmed.len() > MAX_NAME_LEN {
            return Err(NameError::TooLong);
        }
        for (index, label) in trimmed.split('.').enumerate() {
            validate_label(label, index == 0)?;
        }
        Ok(Self {
            text: trimmed.to_ascii_lowercase(),
        })
    }

    /// The name in lowercase dotted form, without the trailing dot.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The labels from leftmost to rightmost.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.text.split('.')
    }

    /// Whether the leftmost label is `*`.
    pub fn is_wildcard(&self) -> bool {
        self.text == "*" || self.text.starts_with("*.")
    }

    /// Whether `self` equals `ancestor` or lies below it.
    pub fn is_within(&self, ancestor: &Self) -> bool {
        is_within(&self.text, &ancestor.text)
    }

    /// Returns `<label>.<self>`, validating the result.
    pub fn child(&self, label: &str) -> Result<Self, NameError> {
        let mut text = String::with_capacity(
            label
                .len()
                .saturating_add(self.text.len())
                .saturating_add(1),
        );
        text.push_str(label);
        text.push('.');
        text.push_str(&self.text);
        Self::new(&text)
    }
}

/// Whether the dotted, lowercased `name` equals `ancestor` or lies below it.
pub(crate) fn is_within(name: &str, ancestor: &str) -> bool {
    match name.strip_suffix(ancestor) {
        Some("") => true,
        Some(rest) => rest.ends_with('.'),
        None => false,
    }
}

/// `name`, then each of its ancestors, ending with the rightmost label.
pub(crate) fn self_and_ancestors(name: &str) -> impl Iterator<Item = &str> {
    core::iter::successors(Some(name), |current| {
        current.split_once('.').map(|(_, rest)| rest)
    })
}

fn validate_label(label: &str, leftmost: bool) -> Result<(), NameError> {
    if label.is_empty() {
        return Err(NameError::EmptyLabel);
    }
    if label.len() > MAX_LABEL_LEN {
        return Err(NameError::LabelTooLong);
    }
    if label == "*" {
        return if leftmost {
            Ok(())
        } else {
            Err(NameError::MisplacedWildcard)
        };
    }
    for byte in label.bytes() {
        match byte {
            b'*' => return Err(NameError::MisplacedWildcard),
            b'-' | b'_' => {}
            other if other.is_ascii_alphanumeric() => {}
            _ => return Err(NameError::InvalidByte),
        }
    }
    if label.starts_with('-') || label.ends_with('-') {
        return Err(NameError::HyphenAtEdge);
    }
    Ok(())
}

impl Borrow<str> for DomainName {
    fn borrow(&self) -> &str {
        &self.text
    }
}

impl AsRef<str> for DomainName {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl FromStr for DomainName {
    type Err = NameError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::new(input)
    }
}

impl TryFrom<&str> for DomainName {
    type Error = NameError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Self::new(input)
    }
}

impl fmt::Display for DomainName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl fmt::Debug for DomainName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DomainName({})", self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_every_malformed_shape_and_normalizes_case() {
        assert_eq!(
            DomainName::new("Laptop.Home.COM.").unwrap().as_str(),
            "laptop.home.com"
        );
        assert!(DomainName::new("_http._tcp.home.com").is_ok());
        for (input, error) in [
            ("", NameError::Empty),
            ("a..b", NameError::EmptyLabel),
            ("a b.c", NameError::InvalidByte),
            ("-a.com", NameError::HyphenAtEdge),
            ("a.*.com", NameError::MisplacedWildcard),
            ("*a.com", NameError::MisplacedWildcard),
        ] {
            assert_eq!(DomainName::new(input), Err(error), "{input}");
        }
        assert_eq!(
            DomainName::new(&"a".repeat(64)),
            Err(NameError::LabelTooLong)
        );
        let max = [
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        ]
        .join(".");
        assert!(DomainName::new(&max).is_ok());
        assert_eq!(
            DomainName::new(&alloc::format!("{max}e")),
            Err(NameError::TooLong)
        );
    }

    #[test]
    fn within_respects_label_boundaries() {
        let apex = DomainName::new("example.com").unwrap();
        assert!(DomainName::new("a.example.com").unwrap().is_within(&apex));
        assert!(apex.is_within(&apex));
        assert!(!DomainName::new("badexample.com").unwrap().is_within(&apex));
    }
}
