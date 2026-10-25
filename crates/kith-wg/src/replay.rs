//! Sliding-window replay filter for transport counters (whitepaper §5.4.6, RFC 6479).

#![warn(clippy::arithmetic_side_effects)]

use crate::time::REJECT_AFTER_MESSAGES;

const WORD_BITS: u64 = 64;
const WORDS: usize = 128;
const WORDS_U64: u64 = 128;

/// How far behind the greatest accepted counter a counter may still be accepted.
pub(crate) const REPLAY_WINDOW: u64 = (WORDS_U64 - 1) * WORD_BITS;

/// Accepts each counter at most once, within [`REPLAY_WINDOW`] of the greatest seen.
///
/// Counters are stored one-based internally, so zero means "nothing accepted yet".
#[derive(Clone, Debug)]
pub(crate) struct ReplayWindow {
    greatest: u64,
    bitmap: [u64; WORDS],
}

impl Default for ReplayWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayWindow {
    /// An empty window.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            greatest: 0,
            bitmap: [0; WORDS],
        }
    }

    /// Whether `counter` would be accepted. Does not record it.
    #[must_use]
    pub(crate) fn check(&self, counter: u64) -> bool {
        if counter >= REJECT_AFTER_MESSAGES {
            return false;
        }
        let position = counter.wrapping_add(1);
        if position.saturating_add(REPLAY_WINDOW) < self.greatest {
            return false;
        }
        if position > self.greatest {
            return true;
        }
        let (word, bit) = Self::locate(position);
        self.bitmap.get(word).is_some_and(|w| w & bit == 0)
    }

    /// Records `counter` if it is acceptable. Call only after the message authenticated,
    /// so forgeries cannot advance the window.
    pub(crate) fn accept(&mut self, counter: u64) -> bool {
        if !self.check(counter) {
            return false;
        }
        let position = counter.wrapping_add(1);
        if position > self.greatest {
            let current_word = self.greatest / WORD_BITS;
            let new_word = position / WORD_BITS;
            let to_clear = new_word.saturating_sub(current_word).min(WORDS_U64);
            for step in 1..=to_clear {
                let (word, _) =
                    Self::locate(current_word.wrapping_add(step).wrapping_mul(WORD_BITS));
                if let Some(slot) = self.bitmap.get_mut(word) {
                    *slot = 0;
                }
            }
            self.greatest = position;
        }
        let (word, bit) = Self::locate(position);
        match self.bitmap.get_mut(word) {
            Some(slot) => {
                *slot |= bit;
                true
            }
            None => false,
        }
    }

    fn locate(position: u64) -> (usize, u64) {
        let word = (position / WORD_BITS) % WORDS_U64;
        let bit = 1_u64 << (position % WORD_BITS);
        (usize::try_from(word).unwrap_or(0), bit)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::collections::BTreeSet;
    use std::vec::Vec;

    use proptest::prelude::*;

    use super::{REPLAY_WINDOW, ReplayWindow};
    use crate::time::REJECT_AFTER_MESSAGES;

    /// Reference: accept iff unseen, below the hard limit and within the window of the max.
    #[derive(Default)]
    struct Model {
        seen: BTreeSet<u64>,
        greatest: Option<u64>,
    }

    impl Model {
        fn accept(&mut self, counter: u64) -> bool {
            let in_window = self
                .greatest
                .is_none_or(|g| counter.saturating_add(REPLAY_WINDOW) >= g);
            if counter >= REJECT_AFTER_MESSAGES || !in_window || !self.seen.insert(counter) {
                return false;
            }
            self.greatest = Some(self.greatest.map_or(counter, |g| g.max(counter)));
            true
        }
    }

    fn counters() -> impl Strategy<Value = Vec<u64>> {
        let step = prop_oneof![
            8 => -70_i64..70,
            2 => -9000_i64..9000,
            1 => Just(i64::from(u32::MAX)),
        ];
        (
            prop_oneof![Just(0_u64), Just(REJECT_AFTER_MESSAGES - 20_000)],
            proptest::collection::vec(step, 1..600),
        )
            .prop_map(|(start, steps)| {
                let mut at = start;
                steps
                    .into_iter()
                    .map(|step| {
                        at = at
                            .saturating_add_signed(step)
                            .min(REJECT_AFTER_MESSAGES + 5);
                        at
                    })
                    .collect()
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn window_matches_reference_model(sequence in counters()) {
            let mut window = ReplayWindow::new();
            let mut model = Model::default();
            for counter in sequence {
                let checked = window.check(counter);
                let accepted = window.accept(counter);
                prop_assert_eq!(checked, accepted);
                prop_assert_eq!(accepted, model.accept(counter), "counter {}", counter);
            }
        }
    }
}
