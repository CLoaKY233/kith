//! The daemon's journal: a numbered, bounded record of what happened, read by `kith log`,
//! `kith watch` and the UI, and the traffic history behind `kith top`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kith_ipc::{Entry, EntryKind, PathKind, PeerInfo, TrafficSample, TrafficSeries};
use kith_types::{NetworkId, NodeId};
use tokio::sync::broadcast;

/// Entries kept for clients that connect later.
const KEEP: usize = 4096;
/// Entries a slow subscriber may fall behind before it misses some.
const LIVE_BUFFER: usize = 1024;
/// Traffic samples kept per peer: an hour at one every two seconds.
const TRAFFIC_SAMPLES: usize = 1800;

/// A shared handle to the journal; clones write to and read from the same one.
#[derive(Debug, Clone)]
pub struct Journal {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: Mutex<State>,
    live: broadcast::Sender<Entry>,
}

#[derive(Debug, Default)]
struct State {
    next: u64,
    entries: VecDeque<Entry>,
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

impl Journal {
    /// An empty journal.
    #[must_use]
    pub fn new() -> Self {
        let (live, _) = broadcast::channel(LIVE_BUFFER);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                live,
            }),
        }
    }

    /// Records `kind` as happening now.
    pub fn push(&self, kind: EntryKind) {
        let mut state = self.lock();
        let entry = Entry {
            seq: state.next,
            at_ms: unix_ms(),
            kind,
        };
        state.next = state.next.saturating_add(1);
        if state.entries.len() >= KEEP {
            state.entries.pop_front();
        }
        state.entries.push_back(entry.clone());
        // Sent under the lock, so a subscriber never sees an entry twice or out of order.
        let _receivers = self.inner.live.send(entry);
    }

    /// Every entry still kept, oldest first.
    #[must_use]
    pub fn entries(&self) -> Vec<Entry> {
        self.lock().entries.iter().cloned().collect()
    }

    /// The kept entries from `since` on, a receiver for every entry after them, and the
    /// sequence number the next entry will get.
    pub(crate) fn subscribe(
        &self,
        since: Option<u64>,
    ) -> (Vec<Entry>, broadcast::Receiver<Entry>, u64) {
        let state = self.lock();
        let live = self.inner.live.subscribe();
        let from = since.unwrap_or(0);
        let backlog = state
            .entries
            .iter()
            .filter(|entry| entry.seq >= from)
            .cloned()
            .collect();
        (backlog, live, state.next)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the daemon last saw of each peer, to turn the next look into journal entries.
#[derive(Debug, Default)]
pub(crate) struct Activity {
    /// Each peer as last seen; `None` before the first look, which records no changes.
    peers: Option<BTreeMap<(NetworkId, NodeId), PeerView>>,
    /// Packet drop counters as last seen.
    drops: BTreeMap<String, u64>,
    /// Traffic samples per peer.
    traffic: BTreeMap<(NetworkId, NodeId), (String, VecDeque<TrafficSample>)>,
}

/// The parts of a peer that the journal follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerView {
    name: String,
    address: String,
    online: bool,
    path: Option<PathKind>,
    gateway: Option<String>,
    endpoint: Option<String>,
}

impl PeerView {
    fn of(info: &PeerInfo) -> Self {
        Self {
            name: info.name.clone(),
            address: info.address.clone(),
            online: info.online,
            path: info.path,
            gateway: info.gateway.clone(),
            endpoint: info.endpoint.clone(),
        }
    }
}

impl Activity {
    /// Takes a new look at every peer and the drop counters, records the changes in
    /// `journal`, and keeps a traffic sample of each peer.
    pub(crate) fn observe(
        &mut self,
        peers: &[(NetworkId, Vec<PeerInfo>)],
        drops: &BTreeMap<String, u64>,
        journal: &Journal,
    ) {
        let now: BTreeMap<(NetworkId, NodeId), PeerView> = peers
            .iter()
            .flat_map(|(network, infos)| {
                infos
                    .iter()
                    .map(|info| ((*network, info.node), PeerView::of(info)))
            })
            .collect();
        if let Some(before) = &self.peers {
            for kind in changes(before, &now) {
                journal.push(kind);
            }
        }
        for (reason, count) in drops {
            let new = count.saturating_sub(self.drops.get(reason).copied().unwrap_or(0));
            if new > 0 && !self.drops.is_empty() {
                journal.push(EntryKind::Dropped {
                    reason: reason.clone(),
                    count: new,
                });
            }
        }
        self.drops.clone_from(drops);
        self.peers = Some(now);

        let at_ms = unix_ms();
        let mut seen = Vec::new();
        for (network, infos) in peers {
            for info in infos {
                let key = (*network, info.node);
                seen.push(key);
                let (name, samples) = self
                    .traffic
                    .entry(key)
                    .or_insert_with(|| (info.name.clone(), VecDeque::new()));
                name.clone_from(&info.name);
                if samples.len() >= TRAFFIC_SAMPLES {
                    samples.pop_front();
                }
                samples.push_back(TrafficSample {
                    at_ms,
                    rx_bytes: info.rx_bytes,
                    tx_bytes: info.tx_bytes,
                });
            }
        }
        self.traffic.retain(|key, _| seen.contains(key));
    }

    /// Bytes per second both ways to `node` in `network` between the last two samples.
    pub(crate) fn rate(&self, network: NetworkId, node: NodeId) -> u64 {
        let Some((_, samples)) = self.traffic.get(&(network, node)) else {
            return 0;
        };
        let mut last = samples.iter().rev();
        let (Some(now), Some(before)) = (last.next(), last.next()) else {
            return 0;
        };
        let bytes = now
            .rx_bytes
            .saturating_add(now.tx_bytes)
            .saturating_sub(before.rx_bytes.saturating_add(before.tx_bytes));
        let millis = now.at_ms.saturating_sub(before.at_ms);
        if millis == 0 {
            return 0;
        }
        u64::try_from(u128::from(bytes) * 1000 / u128::from(millis)).unwrap_or(u64::MAX)
    }

    /// The traffic history of every peer.
    pub(crate) fn traffic(&self) -> Vec<TrafficSeries> {
        self.traffic
            .iter()
            .map(|((network, node), (name, samples))| TrafficSeries {
                network: *network,
                node: *node,
                name: name.clone(),
                samples: samples.iter().copied().collect(),
            })
            .collect()
    }
}

/// The journal entries that turn `before` into `after`.
pub(crate) fn changes(
    before: &BTreeMap<(NetworkId, NodeId), PeerView>,
    after: &BTreeMap<(NetworkId, NodeId), PeerView>,
) -> Vec<EntryKind> {
    let mut kinds = Vec::new();
    for (&(network, node), peer) in after {
        let name = peer.name.clone();
        let Some(old) = before.get(&(network, node)) else {
            kinds.push(EntryKind::DeviceJoined {
                network,
                node,
                name,
                address: peer.address.clone(),
            });
            continue;
        };
        match (old.online, peer.online) {
            (false, true) => kinds.push(EntryKind::PeerOnline {
                network,
                node,
                name,
                path: peer.path,
                gateway: peer.gateway.clone(),
            }),
            (true, false) => kinds.push(EntryKind::PeerOffline {
                network,
                node,
                name,
            }),
            (true, true) if old.path != peer.path => kinds.push(EntryKind::PathChanged {
                network,
                node,
                name,
                from: old.path,
                to: peer.path,
            }),
            (true, true) if old.endpoint.is_some() && peer.endpoint != old.endpoint => {
                if let Some(endpoint) = peer.endpoint.clone() {
                    kinds.push(EntryKind::EndpointChanged {
                        network,
                        node,
                        name,
                        endpoint,
                    });
                }
            }
            _ => {}
        }
    }
    for (&(network, node), peer) in before {
        if !after.contains_key(&(network, node)) {
            kinds.push(EntryKind::DeviceRemoved {
                network,
                node,
                name: peer.name.clone(),
            });
        }
    }
    kinds
}

/// Milliseconds since the Unix epoch.
fn unix_ms() -> u64 {
    crate::channel::now().as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: NetworkId = NetworkId::from_bytes([1; 32]);
    const A: NodeId = NodeId::from_bytes([2; 32]);
    const B: NodeId = NodeId::from_bytes([3; 32]);

    fn view(name: &str, online: bool, path: Option<PathKind>, endpoint: &str) -> PeerView {
        PeerView {
            name: name.into(),
            address: "100.64.0.2".into(),
            online,
            path,
            gateway: None,
            endpoint: Some(endpoint.to_owned()).filter(|e| !e.is_empty()),
        }
    }

    #[test]
    fn a_new_subscriber_gets_history_then_live_entries_once_each() {
        let journal = Journal::new();
        journal.push(EntryKind::Warning {
            message: "one".into(),
        });
        journal.push(EntryKind::Warning {
            message: "two".into(),
        });
        let (backlog, mut live, next) = journal.subscribe(Some(1));
        assert_eq!(backlog.iter().map(|e| e.seq).collect::<Vec<_>>(), [1]);
        assert_eq!(next, 2);
        journal.push(EntryKind::Warning {
            message: "three".into(),
        });
        assert_eq!(live.try_recv().map(|e| e.seq), Ok(2));

        for _ in 0..KEEP {
            journal.push(EntryKind::Warning {
                message: "flood".into(),
            });
        }
        let (backlog, _, _) = journal.subscribe(None);
        assert_eq!(
            backlog.len(),
            KEEP,
            "old entries are dropped, not kept forever"
        );
        assert_eq!(backlog.first().map(|e| e.seq), Some(3));
    }

    #[test]
    fn changes_name_each_transition_once() {
        let before: BTreeMap<_, _> = [
            ((NET, A), view("a", false, None, "")),
            (
                (NET, B),
                view("b", true, Some(PathKind::Direct), "1.2.3.4:5"),
            ),
        ]
        .into();
        assert_eq!(changes(&before, &before), []);

        let moved: BTreeMap<_, _> = [
            ((NET, A), view("a", true, Some(PathKind::Relay), "")),
            (
                (NET, B),
                view("b", true, Some(PathKind::Direct), "5.6.7.8:9"),
            ),
        ]
        .into();
        assert_eq!(
            changes(&before, &moved),
            [
                EntryKind::PeerOnline {
                    network: NET,
                    node: A,
                    name: "a".into(),
                    path: Some(PathKind::Relay),
                    gateway: None,
                },
                EntryKind::EndpointChanged {
                    network: NET,
                    node: B,
                    name: "b".into(),
                    endpoint: "5.6.7.8:9".into(),
                },
            ]
        );

        let after: BTreeMap<_, _> =
            [((NET, A), view("a", true, Some(PathKind::Direct), ""))].into();
        assert_eq!(
            changes(&moved, &after),
            [
                EntryKind::PathChanged {
                    network: NET,
                    node: A,
                    name: "a".into(),
                    from: Some(PathKind::Relay),
                    to: Some(PathKind::Direct),
                },
                EntryKind::DeviceRemoved {
                    network: NET,
                    node: B,
                    name: "b".into()
                },
            ]
        );
    }
}
