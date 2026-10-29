//! Finding the largest packet a path carries by sending probes of chosen sizes, in the
//! spirit of RFC 8899's packetization-layer path MTU discovery: nothing tells a sender
//! when a path silently drops what is too big, so the only way to know is to try.
//!
//! The search tries the target first, since most paths carry it. Once a size is lost it
//! halves the gap between what got through and what did not until they are
//! [`PRECISION`] apart. A size counts as lost only after [`ATTEMPTS`] probes of it went
//! unanswered, so one random loss does not shrink a path. Every [`RECHECK_INTERVAL`] the
//! search runs again, whether it ended at the target or below it, since paths change.
//!
//! Sans-IO: the caller sends the probes, says what was answered or lost, and passes the
//! time in. Sizes are in whatever unit the caller probes in.

#![no_std]
#![forbid(unsafe_code)]

use core::time::Duration;

/// How close the search gets to the real limit before it stops.
pub const PRECISION: u16 = 8;
/// How many probes of one size must be lost before the path is taken not to carry it.
pub const ATTEMPTS: u32 = 2;
/// How often a finished search runs again, since paths change.
pub const RECHECK_INTERVAL: Duration = Duration::from_secs(600);

/// The search for the largest size one path carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SizeSearch {
    target: u16,
    /// The largest size answered.
    good: u16,
    /// The smallest size lost [`ATTEMPTS`] times.
    bad: Option<u16>,
    /// How often the size being tried has been lost so far.
    lost: u32,
    /// Whether a probe is out.
    waiting: bool,
    /// When the search last ended.
    finished_at: Option<Duration>,
}

impl SizeSearch {
    /// A search for whether the path carries `target`, and if not, the largest it does.
    /// A target of zero never probes.
    pub fn new(target: u16) -> Self {
        Self {
            target,
            ..Self::default()
        }
    }

    /// The size the search tries to prove.
    pub const fn target(&self) -> u16 {
        self.target
    }

    /// The largest size that gets through, once the search has found the path does not
    /// carry the target; `None` until then. A guess from halfway through is never given,
    /// since senders remember a limit for minutes and would stay too small. Nor is a
    /// limit given when no probe was ever answered: that says the other end does not
    /// answer probes, not that its path carries nothing.
    pub fn limit(&self) -> Option<u16> {
        self.bad
            .filter(|bad| self.good > 0 && bad.saturating_sub(self.good) <= PRECISION)
            .map(|_| self.good)
    }

    /// When [`Self::next_probe`] next has something, if ever before something changes.
    pub fn next_due(&self, now: Duration) -> Option<Duration> {
        if self.target == 0 || self.waiting {
            return None;
        }
        if self.finished() {
            return Some(
                self.finished_at
                    .map_or(now, |at| at.saturating_add(RECHECK_INTERVAL)),
            );
        }
        Some(now)
    }

    /// Whether the search has its answer: the target got through, or the limit is
    /// within [`PRECISION`].
    fn finished(&self) -> bool {
        self.good >= self.target
            || self
                .bad
                .is_some_and(|bad| bad.saturating_sub(self.good) <= PRECISION)
    }

    /// The size to probe now, if any; the probe counts as out until it is
    /// [`answered`](Self::answered) or [`lost`](Self::lost).
    pub fn next_probe(&mut self, now: Duration) -> Option<u16> {
        if self.target == 0 || self.waiting {
            return None;
        }
        if self.finished() {
            let finished = *self.finished_at.get_or_insert(now);
            if now.saturating_sub(finished) < RECHECK_INTERVAL {
                return None;
            }
            self.finished_at = None;
            self.bad = None;
            if self.good >= self.target {
                self.good = 0;
            }
        }
        let size = match self.bad {
            None => self.target,
            Some(bad) => self.good.midpoint(bad),
        };
        self.waiting = true;
        Some(size)
    }

    /// A probe of `size` was answered: the path carries at least that much.
    pub fn answered(&mut self, size: u16) {
        self.waiting = false;
        self.lost = 0;
        self.good = self.good.max(size);
    }

    /// The probe [`next_probe`](Self::next_probe) gave could not be sent, so it says
    /// nothing about the path; the same size is offered again.
    pub fn not_sent(&mut self) {
        self.waiting = false;
    }

    /// A probe of `size` went unanswered.
    pub fn lost(&mut self, size: u16) {
        self.waiting = false;
        self.lost = self.lost.saturating_add(1);
        if self.lost >= ATTEMPTS {
            self.lost = 0;
            self.bad = Some(self.bad.map_or(size, |bad| bad.min(size)));
        }
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use proptest::prelude::*;

    use super::{PRECISION, RECHECK_INTERVAL, SizeSearch};

    /// Runs a search against a path that carries `carries`, losing every `lose_every`th
    /// probe (0: none), and returns its limit.
    fn run(search: &mut SizeSearch, carries: u16, lose_every: u32, from: Duration) -> Option<u16> {
        let mut sent = 0_u32;
        for step in 0..200_u64 {
            let now = from.saturating_add(Duration::from_secs(step));
            let Some(size) = search.next_probe(now) else {
                continue;
            };
            sent = sent.saturating_add(1);
            if size <= carries && (lose_every == 0 || !sent.is_multiple_of(lose_every)) {
                search.answered(size);
            } else {
                search.lost(size);
            }
        }
        search.limit()
    }

    proptest! {
        #[test]
        fn the_limit_is_within_precision_of_the_truth(carries in 100_u16..1600, target in 1200_u16..1500) {
            let mut search = SizeSearch::new(target);
            let limit = run(&mut search, carries, 0, Duration::ZERO);
            if carries >= target {
                prop_assert_eq!(limit, None);
            } else {
                let limit = limit.unwrap_or(u16::MAX);
                prop_assert!(limit <= carries && carries - limit <= PRECISION, "{} for {}", limit, carries);
            }
        }
    }

    #[test]
    fn a_probe_that_could_not_be_sent_counts_for_nothing() {
        let mut search = SizeSearch::new(1312);
        for _ in 0..10 {
            assert_eq!(search.next_probe(Duration::ZERO), Some(1312));
            search.not_sent();
        }
        assert_eq!(search.limit(), None);
        assert_eq!(
            search.next_probe(Duration::ZERO),
            Some(1312),
            "the target is still the next try"
        );
    }

    #[test]
    fn an_end_that_never_answers_has_no_limit() {
        let mut search = SizeSearch::new(1312);
        assert_eq!(run(&mut search, 0, 0, Duration::ZERO), None);
    }

    #[test]
    fn one_lost_probe_is_not_a_limit() {
        let mut search = SizeSearch::new(1312);
        assert_eq!(run(&mut search, 1500, 2, Duration::ZERO), None);
    }

    #[test]
    fn a_smaller_path_is_checked_again_and_found_to_have_grown() {
        let mut search = SizeSearch::new(1312);
        assert!(run(&mut search, 1248, 0, Duration::ZERO).is_some());
        assert_eq!(search.next_probe(Duration::from_secs(300)), None, "not yet");
        assert_eq!(
            run(
                &mut search,
                1500,
                0,
                RECHECK_INTERVAL + Duration::from_secs(300)
            ),
            None
        );
    }

    #[test]
    fn a_full_path_that_shrinks_later_is_found_at_the_next_check() {
        let mut search = SizeSearch::new(1312);
        assert_eq!(run(&mut search, 1500, 0, Duration::ZERO), None);
        let limit = run(
            &mut search,
            1248,
            0,
            RECHECK_INTERVAL + Duration::from_secs(300),
        );
        assert!(
            limit.is_some_and(|limit| limit <= 1248 && 1248 - limit <= PRECISION),
            "{limit:?}"
        );
    }

    #[test]
    fn nothing_is_probed_for_a_zero_target_or_while_a_probe_is_out() {
        assert_eq!(SizeSearch::new(0).next_probe(Duration::ZERO), None);
        let mut search = SizeSearch::new(1312);
        assert_eq!(search.next_probe(Duration::ZERO), Some(1312));
        assert_eq!(search.next_probe(Duration::ZERO), None);
        assert_eq!(search.next_due(Duration::ZERO), None);
    }
}
