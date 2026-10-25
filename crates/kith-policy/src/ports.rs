//! Port sets: which destination ports a rule opens.
//!
//! Ranges are normalized when built (sorted, overlapping and adjacent ranges merged), and the
//! decoder accepts only that normalized form. So every port set has exactly one encoding, and
//! membership is a binary search.

use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::PolicyError;

/// The most ranges one port set may hold, after normalization.
pub const MAX_PORT_RANGES: usize = 64;

/// An inclusive range of ports, `start..=end`.
///
/// Invariant: `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawPortRange")]
pub struct PortRange {
    start: u16,
    end: u16,
}

impl PortRange {
    /// The range `start..=end`. Fails if `start > end`.
    pub const fn new(start: u16, end: u16) -> Result<Self, PolicyError> {
        if start > end {
            return Err(PolicyError::InvalidPortRange { start, end });
        }
        Ok(Self { start, end })
    }

    /// The range holding only `port`.
    #[must_use]
    pub const fn single(port: u16) -> Self {
        Self {
            start: port,
            end: port,
        }
    }

    /// The first port in the range.
    #[must_use]
    pub const fn start(&self) -> u16 {
        self.start
    }

    /// The last port in the range (inclusive).
    #[must_use]
    pub const fn end(&self) -> u16 {
        self.end
    }

    /// Whether `port` is in the range.
    #[must_use]
    pub const fn contains(&self, port: u16) -> bool {
        self.start <= port && port <= self.end
    }
}

#[derive(Deserialize)]
struct RawPortRange {
    start: u16,
    end: u16,
}

impl TryFrom<RawPortRange> for PortRange {
    type Error = PolicyError;

    fn try_from(raw: RawPortRange) -> Result<Self, Self::Error> {
        Self::new(raw.start, raw.end)
    }
}

/// A non-empty, normalized list of port ranges.
///
/// Invariant: 1 to [`MAX_PORT_RANGES`] ranges, sorted, with a gap of at least one port
/// between consecutive ranges (so no two overlap or touch).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<PortRange>")]
pub struct PortRanges(Vec<PortRange>);

impl PortRanges {
    /// Normalizes `ranges`: sorts them and merges overlapping or adjacent ones.
    ///
    /// Fails if `ranges` is empty or more than [`MAX_PORT_RANGES`] remain after merging.
    pub fn new(mut ranges: Vec<PortRange>) -> Result<Self, PolicyError> {
        if ranges.is_empty() {
            return Err(PolicyError::EmptyPortRanges);
        }
        ranges.sort_unstable();
        let mut merged: Vec<PortRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                // `last.end + 1 >= range.start`, written so it cannot overflow at port 65535.
                Some(last) if range.start <= last.end.saturating_add(1) => {
                    last.end = last.end.max(range.end);
                }
                Some(_) | None => merged.push(range),
            }
        }
        if merged.len() > MAX_PORT_RANGES {
            return Err(PolicyError::TooManyPortRanges {
                count: merged.len(),
            });
        }
        Ok(Self(merged))
    }

    /// The normalized ranges, in ascending order.
    #[must_use]
    pub fn as_slice(&self) -> &[PortRange] {
        &self.0
    }

    /// Whether `port` is in any range. `O(log r)` for `r` ranges; never allocates.
    #[must_use]
    pub fn contains(&self, port: u16) -> bool {
        // The ranges are sorted and disjoint, so the only candidate is the last range that
        // starts at or before `port`.
        let after = self.0.partition_point(|range| range.start <= port);
        after
            .checked_sub(1)
            .and_then(|index| self.0.get(index))
            .is_some_and(|range| range.contains(port))
    }
}

impl TryFrom<Vec<PortRange>> for PortRanges {
    type Error = PolicyError;

    /// Accepts only the already-normalized form, so each set has one encoding.
    fn try_from(ranges: Vec<PortRange>) -> Result<Self, Self::Error> {
        if ranges.len() > MAX_PORT_RANGES {
            return Err(PolicyError::TooManyPortRanges {
                count: ranges.len(),
            });
        }
        let normalized = Self::new(ranges.clone())?;
        if normalized.0 == ranges {
            Ok(normalized)
        } else {
            Err(PolicyError::PortRangesNotNormalized)
        }
    }
}

/// The destination ports a target opens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PortSet {
    /// Every port, and also traffic that has no ports (such as ICMP).
    All,
    /// Only these ports. Traffic without a port never matches.
    Only(PortRanges),
}

impl PortSet {
    /// The set holding only `port`.
    #[must_use]
    pub fn single(port: u16) -> Self {
        Self::Only(PortRanges(alloc::vec![PortRange::single(port)]))
    }

    /// The normalized set of `ranges`. See [`PortRanges::new`].
    pub fn ranges(ranges: Vec<PortRange>) -> Result<Self, PolicyError> {
        PortRanges::new(ranges).map(Self::Only)
    }

    /// Whether a packet with destination port `port` (or none) is in the set.
    #[must_use]
    pub fn matches(&self, port: Option<u16>) -> bool {
        match self {
            Self::All => true,
            Self::Only(ranges) => port.is_some_and(|port| ranges.contains(port)),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{MAX_PORT_RANGES, PortRange, PortRanges, PortSet};
    use crate::PolicyError;

    fn range(start: u16, end: u16) -> PortRange {
        PortRange::new(start, end).unwrap()
    }

    #[test]
    fn merges_overlapping_and_adjacent_ranges() {
        let set = PortRanges::new(vec![
            range(100, 200),
            range(10, 20),
            range(150, 300),
            range(21, 30),
            range(32, 32),
            range(65000, 65535),
            range(65535, 65535),
        ])
        .unwrap();
        assert_eq!(
            set.as_slice(),
            [
                range(10, 30),
                range(32, 32),
                range(100, 300),
                range(65000, 65535)
            ]
        );
    }

    #[test]
    fn membership() {
        let set = PortSet::ranges(vec![range(22, 22), range(80, 90)]).unwrap();
        for (port, expected) in [
            (21, false),
            (22, true),
            (23, false),
            (79, false),
            (80, true),
            (85, true),
            (90, true),
            (91, false),
        ] {
            assert_eq!(set.matches(Some(port)), expected, "port {port}");
        }
        assert!(!set.matches(None));
        assert!(PortSet::All.matches(None));
        assert!(PortSet::single(0).matches(Some(0)));
    }

    #[test]
    fn validation_failures() {
        assert_eq!(
            PortRange::new(9, 8),
            Err(PolicyError::InvalidPortRange { start: 9, end: 8 })
        );
        assert_eq!(
            PortRanges::new(Vec::new()),
            Err(PolicyError::EmptyPortRanges)
        );
        let many: Vec<_> = (0..=u16::try_from(MAX_PORT_RANGES).unwrap())
            .map(|index| PortRange::single(index * 2))
            .collect();
        assert_eq!(
            PortRanges::new(many),
            Err(PolicyError::TooManyPortRanges {
                count: MAX_PORT_RANGES + 1
            })
        );
        // Unnormalized input is accepted by the constructor but not by the decoder.
        assert_eq!(
            PortRanges::try_from(vec![range(5, 6), range(1, 2)]),
            Err(PolicyError::PortRangesNotNormalized)
        );
        assert_eq!(
            PortRanges::try_from(vec![range(1, 2), range(3, 4)]),
            Err(PolicyError::PortRangesNotNormalized)
        );
        assert!(PortRanges::try_from(vec![range(1, 2), range(4, 4)]).is_ok());
    }
}
