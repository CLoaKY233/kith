//! Wall-clock time as plain data. Cores never read the clock; drivers pass `now` in.

use core::fmt;
use core::time::Duration;

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch (UTC). Never used to order events, since device clocks disagree.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The Unix epoch.
    pub const EPOCH: Self = Self(0);

    /// The latest representable instant.
    pub const MAX: Self = Self(u64::MAX);

    /// From milliseconds since the Unix epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// `self + duration`, or `None` on overflow.
    #[must_use]
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let millis = u64::try_from(duration.as_millis()).ok()?;
        self.0.checked_add(millis).map(Self)
    }

    /// `self + duration`, clamped to [`Timestamp::MAX`].
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        self.checked_add(duration).unwrap_or(Self::MAX)
    }

    /// Time elapsed from `earlier` to `self`, or zero if `earlier` is later.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({}ms)", self.0)
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::Timestamp;

    #[test]
    fn arithmetic_never_wraps() {
        let near_end = Timestamp::from_millis(u64::MAX - 1);
        assert_eq!(near_end.checked_add(Duration::from_millis(2)), None);
        assert_eq!(
            near_end.saturating_add(Duration::from_secs(1)),
            Timestamp::MAX
        );
        assert_eq!(
            Timestamp::EPOCH.saturating_duration_since(near_end),
            Duration::ZERO
        );
    }

    #[test]
    fn huge_durations_do_not_truncate() {
        assert_eq!(Timestamp::EPOCH.checked_add(Duration::MAX), None);
    }
}
