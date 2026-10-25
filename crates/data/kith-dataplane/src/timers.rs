//! Per-peer deadlines in a min-heap with lazy invalidation; a deadline only moves earlier.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::time::Duration;

const COMPACTION_SLACK: usize = 64;

pub(crate) struct Timers<K> {
    heap: BinaryHeap<Reverse<(Duration, u64, K)>>,
    live: BTreeMap<K, (Duration, u64)>,
    next_sequence: u64,
}

impl<K: Ord + Copy> Timers<K> {
    pub(crate) fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            live: BTreeMap::new(),
            next_sequence: 0,
        }
    }

    pub(crate) fn schedule(&mut self, key: K, due: Duration) {
        if self
            .live
            .get(&key)
            .is_some_and(|&(current, _)| current <= due)
        {
            return;
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence.wrapping_add(1);
        self.live.insert(key, (due, sequence));
        self.heap.push(Reverse((due, sequence, key)));
        if self.heap.len() > self.live.len().saturating_mul(2) + COMPACTION_SLACK {
            let live = &self.live;
            self.heap.retain(|entry| is_live(live, entry));
        }
    }

    pub(crate) fn cancel(&mut self, key: K) {
        self.live.remove(&key);
    }

    pub(crate) fn pop_due(&mut self, now: Duration) -> Option<K> {
        while let Some(entry @ Reverse((due, _, key))) = self.heap.peek().copied() {
            if due > now {
                return None;
            }
            self.heap.pop();
            if is_live(&self.live, &entry) {
                self.live.remove(&key);
                return Some(key);
            }
        }
        None
    }

    pub(crate) fn next_deadline(&mut self) -> Option<Duration> {
        while let Some(entry @ Reverse((due, _, _))) = self.heap.peek().copied() {
            if is_live(&self.live, &entry) {
                return Some(due);
            }
            self.heap.pop();
        }
        None
    }
}

fn is_live<K: Ord>(
    live: &BTreeMap<K, (Duration, u64)>,
    Reverse((_, sequence, key)): &Reverse<(Duration, u64, K)>,
) -> bool {
    live.get(key)
        .is_some_and(|(_, current)| current == sequence)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use proptest::prelude::*;

    use super::{COMPACTION_SLACK, Timers};

    const KEYS: u8 = 16;

    #[derive(Debug, Clone)]
    enum Op {
        Schedule(u8, u16),
        Pull(u8),
        Cancel(u8),
        Advance(u16),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => (0..KEYS, any::<u16>()).prop_map(|(key, delay)| Op::Schedule(key, delay)),
            4 => (0..KEYS).prop_map(Op::Pull),
            1 => (0..KEYS).prop_map(Op::Cancel),
            1 => (0_u16..1000).prop_map(Op::Advance),
        ]
    }

    proptest! {
        #[test]
        fn timers_fire_every_deadline_on_time_and_stay_bounded(
            ops in proptest::collection::vec(op(), 1..600),
        ) {
            let mut timers = Timers::new();
            let mut deadlines: BTreeMap<u8, Duration> = BTreeMap::new();
            let mut now = Duration::ZERO;
            for op in ops {
                match op {
                    Op::Schedule(key, delay) => {
                        let due = now + Duration::from_millis(u64::from(delay));
                        deadlines.insert(key, due);
                        timers.schedule(key, due);
                    }
                    Op::Pull(key) => {
                        if let Some(due) = deadlines.get_mut(&key) {
                            *due = now + due.saturating_sub(now) / 2;
                            timers.schedule(key, *due);
                        }
                    }
                    Op::Cancel(key) => {
                        deadlines.remove(&key);
                        timers.cancel(key);
                    }
                    Op::Advance(delay) => {
                        let until = now + Duration::from_millis(u64::from(delay));
                        while let Some(next) = timers.next_deadline().filter(|due| *due <= until) {
                            now = next;
                            while let Some(key) = timers.pop_due(now) {
                                let due = deadlines.get(&key).copied();
                                prop_assert!(due.is_some(), "fired an unscheduled key");
                                match due {
                                    Some(due) if due > now => timers.schedule(key, due),
                                    _ => {
                                        deadlines.remove(&key);
                                    }
                                }
                            }
                        }
                        now = until;
                        prop_assert!(deadlines.values().all(|due| *due > now), "a deadline was missed");
                    }
                }
                let next = timers.next_deadline();
                prop_assert_eq!(next.is_some(), !deadlines.is_empty());
                prop_assert!(next <= deadlines.values().min().copied(), "would sleep past a deadline");
                prop_assert!(timers.heap.len() <= 2 * usize::from(KEYS) + COMPACTION_SLACK + 1);
            }
        }
    }
}
