//! Per-device traffic history: an hour of samples at one every two seconds, so the page's
//! hour chart survives a daemon that keeps less.

use std::collections::VecDeque;

use kith_ipc::TrafficSample;

/// Samples kept per device: an hour at one every two seconds.
pub const CAPACITY: usize = 1800;

/// The newest [`CAPACITY`] samples of one device, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Ring {
    samples: VecDeque<TrafficSample>,
}

impl Ring {
    /// Adds a sample, dropping the oldest when full. A sample not newer than the last is
    /// ignored, so replays after a reconnect add nothing twice.
    pub fn push(&mut self, sample: TrafficSample) {
        if self
            .samples
            .back()
            .is_some_and(|last| last.at_ms >= sample.at_ms)
        {
            return;
        }
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// The newest `count` samples, oldest first.
    pub fn tail(&self, count: usize) -> impl Iterator<Item = &TrafficSample> {
        self.samples
            .iter()
            .skip(self.samples.len().saturating_sub(count))
    }

    /// How many samples are kept.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no sample is kept.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: u64) -> TrafficSample {
        TrafficSample {
            at_ms,
            rx_bytes: at_ms,
            tx_bytes: 0,
        }
    }

    #[test]
    fn keeps_an_hour_in_order_and_ignores_replays() {
        let mut ring = Ring::default();
        for at in 1..=2000 {
            ring.push(sample(at * 2000));
        }
        ring.push(sample(10));
        assert_eq!(ring.len(), CAPACITY);
        let tail: Vec<u64> = ring.tail(2).map(|s| s.at_ms).collect();
        assert_eq!(tail, [3_998_000, 4_000_000]);
    }
}
