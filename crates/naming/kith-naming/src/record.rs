//! Resource records users and devices can publish.
//!
//! Only the types kith serves are representable, and each is validated on construction
//! so the answer engine can write any stored record without re-checking it.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use crate::name::DomainName;

/// Longest TXT character-string (RFC 1035 §3.3: one length byte).
pub const MAX_TXT_STRING_LEN: usize = 255;

/// Most character-strings in one TXT record. Keeps one record well under a datagram.
pub(crate) const MAX_TXT_STRINGS: usize = 16;

/// Largest TTL (RFC 2181 §8: TTLs are 31-bit).
pub(crate) const MAX_TTL: u32 = 0x7FFF_FFFF;

/// Why a record is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecordError {
    /// A TXT record needs at least one character-string.
    #[error("TXT record has no strings")]
    EmptyTxt,
    /// More than `MAX_TXT_STRINGS` character-strings.
    #[error("TXT record has more than {} strings", MAX_TXT_STRINGS)]
    TooManyTxtStrings,
    /// A character-string longer than [`MAX_TXT_STRING_LEN`] bytes.
    #[error("TXT string is longer than {} bytes", MAX_TXT_STRING_LEN)]
    TxtStringTooLong,
    /// TTL above `MAX_TTL`.
    #[error("TTL {0} exceeds {max}", max = MAX_TTL)]
    TtlTooLarge(u32),
    /// A CNAME or SRV target is a wildcard name, which can never be resolved.
    #[error("record target is a wildcard name")]
    WildcardTarget,
}

/// The record types the answer engine knows, with their RFC 1035 / 3596 / 2782 codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RecordType {
    /// IPv4 address (1).
    A,
    /// Start of authority (6), synthesized per authoritative suffix.
    Soa,
    /// Canonical name (5).
    Cname,
    /// Reverse pointer (12), synthesized for device addresses.
    Ptr,
    /// Text (16).
    Txt,
    /// IPv6 address (28).
    Aaaa,
    /// Service locator (33).
    Srv,
}

impl RecordType {
    /// The wire code.
    pub const fn code(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Cname => 5,
            Self::Soa => 6,
            Self::Ptr => 12,
            Self::Txt => 16,
            Self::Aaaa => 28,
            Self::Srv => 33,
        }
    }
}

/// The character-strings of a TXT record.
///
/// Invariant: 1 to `MAX_TXT_STRINGS` strings, each at most [`MAX_TXT_STRING_LEN`]
/// bytes of UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Txt {
    strings: Vec<String>,
}

impl Txt {
    /// Validates the strings.
    pub fn new(strings: Vec<String>) -> Result<Self, RecordError> {
        if strings.is_empty() {
            return Err(RecordError::EmptyTxt);
        }
        if strings.len() > MAX_TXT_STRINGS {
            return Err(RecordError::TooManyTxtStrings);
        }
        if strings.iter().any(|text| text.len() > MAX_TXT_STRING_LEN) {
            return Err(RecordError::TxtStringTooLong);
        }
        Ok(Self { strings })
    }

    /// The strings, in order.
    pub fn strings(&self) -> &[String] {
        &self.strings
    }
}

/// Service locator data (RFC 2782). Plain data: every field combination is valid.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Srv {
    /// Lower is preferred.
    pub priority: u16,
    /// Relative weight among equal priorities.
    pub weight: u16,
    /// Service port.
    pub port: u16,
    /// Host providing the service.
    pub target: DomainName,
}

/// Record data a user or device can publish.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordData {
    /// IPv4 address.
    A(Ipv4Addr),
    /// IPv6 address.
    Aaaa(Ipv6Addr),
    /// Alias to another name. Must be the only record at its name.
    Cname(DomainName),
    /// Text strings.
    Txt(Txt),
    /// Service locator.
    Srv(Srv),
}

impl RecordData {
    /// The record type.
    pub const fn record_type(&self) -> RecordType {
        match self {
            Self::A(_) => RecordType::A,
            Self::Aaaa(_) => RecordType::Aaaa,
            Self::Cname(_) => RecordType::Cname,
            Self::Txt(_) => RecordType::Txt,
            Self::Srv(_) => RecordType::Srv,
        }
    }
}

/// A record with its TTL.
///
/// Invariants: TTL ≤ `MAX_TTL`; CNAME and SRV targets are not wildcards.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Record {
    // Field order matters: records sort by data first, so an RRset is contiguous.
    data: RecordData,
    ttl: u32,
}

impl Record {
    /// Validates and builds a record.
    pub fn new(ttl: u32, data: RecordData) -> Result<Self, RecordError> {
        if ttl > MAX_TTL {
            return Err(RecordError::TtlTooLarge(ttl));
        }
        let target = match &data {
            RecordData::Cname(target) | RecordData::Srv(Srv { target, .. }) => Some(target),
            RecordData::A(_) | RecordData::Aaaa(_) | RecordData::Txt(_) => None,
        };
        if target.is_some_and(DomainName::is_wildcard) {
            return Err(RecordError::WildcardTarget);
        }
        Ok(Self { data, ttl })
    }

    /// The TTL in seconds.
    pub const fn ttl(&self) -> u32 {
        self.ttl
    }

    /// The record data.
    pub const fn data(&self) -> &RecordData {
        &self.data
    }

    /// The record type.
    pub const fn record_type(&self) -> RecordType {
        self.data.record_type()
    }

    pub(crate) fn with_ttl(&self, ttl: u32) -> Self {
        Self {
            data: self.data.clone(),
            ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn constructors_enforce_txt_ttl_and_target_bounds() {
        assert_eq!(Txt::new(vec![]), Err(RecordError::EmptyTxt));
        assert_eq!(
            Txt::new(vec!["x".repeat(256)]),
            Err(RecordError::TxtStringTooLong)
        );
        assert!(Txt::new(vec!["x".repeat(255)]).is_ok());
        assert_eq!(
            Txt::new(vec![String::new(); MAX_TXT_STRINGS + 1]),
            Err(RecordError::TooManyTxtStrings)
        );
        let a = RecordData::A(Ipv4Addr::LOCALHOST);
        assert_eq!(
            Record::new(MAX_TTL + 1, a),
            Err(RecordError::TtlTooLarge(MAX_TTL + 1))
        );
        let wildcard = DomainName::new("*.example.com").unwrap();
        assert_eq!(
            Record::new(60, RecordData::Cname(wildcard)),
            Err(RecordError::WildcardTarget)
        );
    }
}
