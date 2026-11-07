//! The daemon's journal: a numbered, bounded record of what happened, read by `kith log`,
//! `kith watch` and the UI, and the traffic history behind `kith top`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
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
/// Bytes of free text, such as a log message, an entry keeps; the rest is cut off so a
/// huge log line cannot make every kept and buffered copy of it huge too.
const MAX_TEXT: usize = 2048;

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
    /// Where entries are kept across restarts, and how many lines the file holds.
    file: Option<(PathBuf, usize)>,
}

/// Entries reloaded after a restart; the file is cut back to this many when it doubles.
const PERSIST: usize = 512;

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

    /// Reloads the entries kept in `path` by an earlier run and keeps every later entry
    /// there too, bounded to the last few hundred. A missing or damaged file starts empty.
    pub fn persist(&self, path: &Path) {
        let mut state = self.lock();
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let loaded: Vec<Entry> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let start = loaded.len().saturating_sub(PERSIST);
        let mut kept: VecDeque<Entry> = loaded.into_iter().skip(start).collect();
        let mut next = kept.back().map_or(0, |entry| entry.seq.saturating_add(1));
        for mut entry in state.entries.drain(..) {
            entry.seq = next;
            next = next.saturating_add(1);
            kept.push_back(entry);
        }
        state.next = next;
        state.entries = kept;
        let lines = state.entries.len();
        state.file = Some((path.to_owned(), lines));
        rewrite(path, state.entries.iter());
    }

    /// Records `kind` as happening now.
    pub fn push(&self, mut kind: EntryKind) {
        clip_text(&mut kind);
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
        save(&mut state, &entry);
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

/// Appends `entry` to the journal file, if any, cutting it back once it doubles.
fn save(state: &mut State, entry: &Entry) {
    let Some((path, lines)) = state.file.as_mut() else {
        return;
    };
    *lines = lines.saturating_add(1);
    if *lines > PERSIST.saturating_mul(2) {
        *lines = PERSIST;
        let start = state.entries.len().saturating_sub(PERSIST);
        rewrite(path, state.entries.iter().skip(start));
        return;
    }
    let Ok(line) = serde_json::to_string(entry) else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    if let Ok(mut file) = file {
        let _ignored = writeln!(file, "{line}");
    }
}

/// Replaces the journal file with `entries`, one JSON object per line.
fn rewrite<'a>(path: &Path, entries: impl Iterator<Item = &'a Entry>) {
    let mut text = String::new();
    for entry in entries {
        if let Ok(line) = serde_json::to_string(entry) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    let temp = path.with_extension("tmp");
    if std::fs::write(&temp, text).is_ok() {
        let _ignored = std::fs::rename(&temp, path);
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
    invite: Option<String>,
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
            invite: info.invite.clone(),
        }
    }
}

impl Activity {
    /// Takes a new look at every peer and the drop counters, records the changes in
    /// `journal`, and keeps a traffic sample of each peer.
    /// Takes a look at a moment no network has any peer: a device that was there is
    /// recorded as removed, and the first device to join later is recorded as joining
    /// rather than taken as the starting point.
    pub(crate) fn note_no_peers(&mut self, journal: &Journal) {
        let empty = BTreeMap::new();
        if let Some(before) = &self.peers {
            for kind in changes(before, &empty) {
                journal.push(kind);
            }
        }
        self.peers = Some(empty);
        self.traffic.clear();
    }

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
            // The first look only sets the baseline; after it, a reason seen for the first
            // time counts from zero.
            if new > 0 && self.peers.is_some() {
                journal.push(EntryKind::Dropped {
                    reason: reason.clone(),
                    count: new,
                });
            }
        }
        self.drops.clone_from(drops);
        self.peers = Some(now);

        let at_ms = unix_ms();
        let mut seen = BTreeSet::new();
        for (network, infos) in peers {
            for info in infos {
                let key = (*network, info.node);
                seen.insert(key);
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
                invite: peer.invite.clone(),
            });
            if peer.online {
                kinds.push(EntryKind::PeerOnline {
                    network,
                    node,
                    name: peer.name.clone(),
                    path: peer.path,
                    gateway: peer.gateway.clone(),
                });
            }
            continue;
        };
        if old.name != peer.name {
            kinds.push(EntryKind::DeviceRenamed {
                network,
                node,
                old_name: old.name.clone(),
                name: name.clone(),
            });
        }
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

/// Cuts the free text of `kind` down to [`MAX_TEXT`] bytes.
fn clip_text(kind: &mut EntryKind) {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "only these entries carry text of any length"
    )]
    match kind {
        EntryKind::Log {
            target, message, ..
        } => {
            clip(target);
            clip(message);
        }
        EntryKind::Warning { message } => clip(message),
        _ => {}
    }
}

/// Cuts `text` to at most [`MAX_TEXT`] bytes on a character boundary, marking the cut.
fn clip(text: &mut String) {
    if text.len() > MAX_TEXT {
        let end = text.floor_char_boundary(MAX_TEXT.saturating_sub('…'.len_utf8()));
        text.truncate(end);
        text.push('…');
    }
}

/// Milliseconds since the Unix epoch.
pub(crate) fn unix_ms() -> u64 {
    crate::channel::now().as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: NetworkId = NetworkId::from_bytes([1; 32]);
    const A: NodeId = NodeId::from_bytes([2; 32]);
    const B: NodeId = NodeId::from_bytes([3; 32]);

    #[test]
    fn journal_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("kith-journal-{}", std::process::id()));
        drop(std::fs::create_dir_all(&dir));
        let path = dir.join("journal.jsonl");
        drop(std::fs::remove_file(&path));
        let first = Journal::new();
        first.persist(&path);
        first.push(EntryKind::DeviceRemoved {
            network: NET,
            node: A,
            name: "a".into(),
        });
        first.push(EntryKind::DeviceRemoved {
            network: NET,
            node: B,
            name: "b".into(),
        });

        let second = Journal::new();
        second.persist(&path);
        second.push(EntryKind::DeviceRemoved {
            network: NET,
            node: A,
            name: "c".into(),
        });
        let seqs: Vec<u64> = second.entries().iter().map(|entry| entry.seq).collect();
        assert_eq!(seqs, [0, 1, 2]);
        drop(std::fs::remove_dir_all(&dir));
    }

    fn view(name: &str, online: bool, path: Option<PathKind>, endpoint: &str) -> PeerView {
        PeerView {
            name: name.into(),
            address: "100.64.0.2".into(),
            online,
            path,
            gateway: None,
            endpoint: Some(endpoint.to_owned()).filter(|e| !e.is_empty()),
            invite: None,
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
    fn long_text_is_cut_on_a_character_boundary() {
        let journal = Journal::new();
        journal.push(EntryKind::Log {
            level: "info".into(),
            target: "kith".into(),
            message: "é".repeat(MAX_TEXT),
        });
        let entries = journal.entries();
        let Some(EntryKind::Log { message, .. }) = entries.first().map(|e| &e.kind) else {
            panic!("the log entry was not kept");
        };
        assert!(message.len() <= MAX_TEXT, "{} bytes kept", message.len());
        assert!(message.ends_with('…'));
    }

    #[test]
    fn drops_count_after_the_first_look_even_for_a_new_reason() {
        let journal = Journal::new();
        let mut activity = Activity::default();
        let peers = [(NET, Vec::new())];
        activity.observe(&peers, &[("ttl".to_owned(), 9)].into(), &journal);
        assert!(
            journal.entries().is_empty(),
            "the first look is the baseline"
        );

        activity.observe(&peers, &BTreeMap::new(), &journal);
        activity.observe(&peers, &[("policy".to_owned(), 3)].into(), &journal);
        let kinds: Vec<_> = journal.entries().into_iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [EntryKind::Dropped {
                reason: "policy".into(),
                count: 3
            }]
        );
    }

    #[test]
    fn the_first_device_in_an_empty_network_is_recorded_as_joining() {
        let journal = Journal::new();
        let mut activity = Activity::default();
        activity.note_no_peers(&journal);
        let info = PeerInfo {
            node: A,
            name: "laptop".into(),
            address: "100.64.0.2".into(),
            path: None,
            path_reason: None,
            last_seen_secs: None,
            endpoint: None,
            rx_bytes: 0,
            tx_bytes: 0,
            online: false,
            display_name: "laptop".into(),
            gateway: None,
            path_mtu: None,
            invite: None,
        };
        activity.observe(&[(NET, vec![info])], &BTreeMap::new(), &journal);
        assert!(
            journal.entries().iter().any(
                |e| matches!(&e.kind, EntryKind::DeviceJoined { name, .. } if name == "laptop")
            ),
            "{:?}",
            journal.entries()
        );

        activity.note_no_peers(&journal);
        assert!(
            journal.entries().iter().any(
                |e| matches!(&e.kind, EntryKind::DeviceRemoved { name, .. } if name == "laptop")
            ),
            "the last device leaving is recorded too: {:?}",
            journal.entries()
        );
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
