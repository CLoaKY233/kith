use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, PoisonError, RwLock};

use kith_naming::{SplitDns, UpstreamGroupId, Zone};

/// One consistent view of everything a query needs.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) zone: Zone,
    pub(crate) split: SplitDns,
    pub(crate) groups: BTreeMap<UpstreamGroupId, Vec<SocketAddr>>,
    pub(crate) default_upstreams: Vec<SocketAddr>,
}

impl Snapshot {
    /// The upstreams for a split-DNS group; an unknown group has none.
    pub(crate) fn group(&self, id: UpstreamGroupId) -> &[SocketAddr] {
        self.groups.get(&id).map_or(&[], Vec::as_slice)
    }
}

/// Hot-swappable resolver state shared by the server and its owner.
///
/// Each setter swaps in a new snapshot; a query in flight keeps the one it started with.
#[derive(Debug)]
pub struct DnsState {
    current: RwLock<Arc<Snapshot>>,
}

impl DnsState {
    /// State that answers from `zone` and forwards nothing until upstreams are set.
    pub fn new(zone: Zone) -> Self {
        Self {
            current: RwLock::new(Arc::new(Snapshot {
                zone,
                split: SplitDns::new(),
                groups: BTreeMap::new(),
                default_upstreams: Vec::new(),
            })),
        }
    }

    /// Replaces the authoritative zone.
    pub fn set_zone(&self, zone: Zone) {
        self.update(|snapshot| snapshot.zone = zone);
    }

    /// Replaces the split-DNS routes together with the upstreams of each group.
    pub fn set_split(&self, split: SplitDns, groups: BTreeMap<UpstreamGroupId, Vec<SocketAddr>>) {
        self.update(|snapshot| {
            snapshot.split = split;
            snapshot.groups = groups;
        });
    }

    /// Replaces the upstreams used for names no split route covers.
    pub fn set_default_upstreams(&self, upstreams: Vec<SocketAddr>) {
        self.update(|snapshot| snapshot.default_upstreams = upstreams);
    }

    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut guard = self.current.write().unwrap_or_else(PoisonError::into_inner);
        change(Arc::make_mut(&mut guard));
    }
}
