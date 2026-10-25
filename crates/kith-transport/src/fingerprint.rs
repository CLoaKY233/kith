//! Network identity and the per-network memory of which rung worked.
//!
//! Blocking is a property of the network, not the peer, so the winning rung is
//! cached per network to skip the stagger delays next time.

use alloc::vec::Vec;
use core::{fmt, time::Duration};

use crate::TransportKind;

/// Opaque identifier of the local network the device is attached to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkFingerprint(pub [u8; 32]);

impl fmt::Debug for NetworkFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkFingerprint(")?;
        for byte in self.0.iter().take(8) {
            write!(f, "{byte:02x}")?;
        }
        f.write_str("…)")
    }
}

/// One remembered network.
#[derive(Debug, Clone, Copy)]
struct Entry {
    fingerprint: NetworkFingerprint,
    kind: TransportKind,
    stored_at: Duration,
}

/// Bounded least-recently-used map from network to winning rung, with a TTL.
///
/// The `Vec` is in recency order, least recent first; at this size a linear scan beats a map.
#[derive(Debug, Clone)]
pub(crate) struct NetworkCache {
    entries: Vec<Entry>,
    capacity: usize,
    ttl: Duration,
}

impl NetworkCache {
    pub(crate) fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            ttl,
        }
    }

    /// The fresh rung remembered for `fingerprint`, marking it recently used.
    /// Expired entries are dropped.
    pub(crate) fn lookup(
        &mut self,
        now: Duration,
        fingerprint: NetworkFingerprint,
    ) -> Option<TransportKind> {
        let position = self
            .entries
            .iter()
            .position(|entry| entry.fingerprint == fingerprint)?;
        let entry = self.entries.remove(position);
        if now.saturating_sub(entry.stored_at) >= self.ttl {
            return None;
        }
        self.entries.push(entry);
        Some(entry.kind)
    }

    /// Remembers `kind` as the winner on `fingerprint`, evicting the least
    /// recently used entry when full.
    pub(crate) fn store(
        &mut self,
        now: Duration,
        fingerprint: NetworkFingerprint,
        kind: TransportKind,
    ) {
        if self.capacity == 0 {
            return;
        }
        self.entries
            .retain(|entry| entry.fingerprint != fingerprint);
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push(Entry {
            fingerprint,
            kind,
            stored_at: now,
        });
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(byte: u8) -> NetworkFingerprint {
        NetworkFingerprint([byte; 32])
    }

    #[test]
    fn full_cache_evicts_least_recently_used_not_oldest() {
        let mut cache = NetworkCache::new(2, Duration::from_secs(100));
        let now = Duration::ZERO;
        cache.store(now, network(1), TransportKind::Quic);
        cache.store(now, network(2), TransportKind::WebSocket);
        assert_eq!(cache.lookup(now, network(1)), Some(TransportKind::Quic));
        cache.store(now, network(3), TransportKind::Udp);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.lookup(now, network(2)), None);
        assert_eq!(cache.lookup(now, network(1)), Some(TransportKind::Quic));
        assert_eq!(cache.lookup(now, network(3)), Some(TransportKind::Udp));
    }

    #[test]
    fn expired_entry_is_forgotten() {
        let mut cache = NetworkCache::new(2, Duration::from_secs(10));
        cache.store(Duration::ZERO, network(1), TransportKind::Quic);
        assert_eq!(cache.lookup(Duration::from_secs(10), network(1)), None);
        assert_eq!(cache.len(), 0);
    }
}
