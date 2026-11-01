//! Per-device traffic history: an hour of samples at one every two seconds, kept by the page
//! server so the hour chart survives a daemon that keeps less or restarts.

use std::collections::{BTreeMap, VecDeque};

use kith_ipc::{TrafficSample, TrafficSeries};
use kith_types::{NetworkId, NodeId};

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

    /// The newest sample's time, if any.
    pub fn last_ms(&self) -> Option<u64> {
        self.samples.back().map(|sample| sample.at_ms)
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

/// One device's name and samples.
#[derive(Debug, Clone, Default)]
struct Device {
    name: String,
    ring: Ring,
}

/// A [`Ring`] per device, for every device seen since the server started.
#[derive(Debug, Clone, Default)]
pub struct History {
    devices: BTreeMap<(NetworkId, NodeId), Device>,
}

impl History {
    /// Merges what the daemon reports: only samples newer than those kept are added, so
    /// the daemon's full hour can be passed on every tick.
    pub fn record(&mut self, series: &[TrafficSeries]) {
        for one in series {
            let device = self.devices.entry((one.network, one.node)).or_default();
            device.name.clone_from(&one.name);
            let newest = device.ring.last_ms();
            for sample in &one.samples {
                if newest.is_none_or(|last| sample.at_ms > last) {
                    device.ring.push(*sample);
                }
            }
        }
    }

    /// Adds one sample for a device already named by [`History::record`]; an unknown
    /// device is kept without a name.
    pub fn add(&mut self, network: NetworkId, node: NodeId, sample: TrafficSample) {
        self.devices
            .entry((network, node))
            .or_default()
            .ring
            .push(sample);
    }

    /// Every device's newest `count` samples, ordered by network and device.
    pub fn series(&self, count: usize) -> Vec<TrafficSeries> {
        self.devices
            .iter()
            .map(|((network, node), device)| TrafficSeries {
                network: *network,
                node: *node,
                name: device.name.clone(),
                samples: device.ring.tail(count).copied().collect(),
            })
            .collect()
    }

    /// Whether nothing was recorded yet.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
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

    #[test]
    fn history_outlives_a_daemon_that_keeps_less() {
        let network = NetworkId::from_bytes([1; 32]);
        let node = NodeId::from_bytes([2; 32]);
        let report = |samples: Vec<TrafficSample>| TrafficSeries {
            network,
            node,
            name: "nas".into(),
            samples,
        };
        let mut history = History::default();
        history.record(&[report((1..=3).map(|at| sample(at * 2000)).collect())]);
        history.record(&[report((3..=5).map(|at| sample(at * 2000)).collect())]);
        history.add(network, node, sample(12_000));
        let series = history.series(CAPACITY);
        let times: Vec<u64> = series
            .iter()
            .flat_map(|one| one.samples.iter().map(|s| s.at_ms))
            .collect();
        assert_eq!(times, [2000, 4000, 6000, 8000, 10_000, 12_000]);
        assert_eq!(series.first().map(|one| one.name.as_str()), Some("nas"));
        assert_eq!(
            history
                .series(2)
                .iter()
                .map(|one| one.samples.len())
                .sum::<usize>(),
            2
        );
    }
}
