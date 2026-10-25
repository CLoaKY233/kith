//! The connection table: a fixed-size, open-addressing hash table.
//!
//! Linear probing over a power-of-two slot array that is at most half full, with
//! backward-shift deletion so no tombstones accumulate. All memory is allocated by
//! [`Table::new`]; lookups, inserts and removals never allocate.

use alloc::boxed::Box;
use alloc::vec;
use core::net::IpAddr;
use core::time::Duration;

use crate::timeouts::{
    ICMP_ECHO, TCP_CLOSE, TCP_CLOSING, TCP_ESTABLISHED, TCP_SYN_SENT, UDP_STREAM, UDP_UNREPLIED,
};

/// How many slots from a key's home are searched for a victim when the table is full.
///
/// Mirrors Linux's `NF_CT_EVICTION_RANGE` (8), used by `early_drop` in
/// `net/netfilter/nf_conntrack_core.c`: a bounded scan keeps a full table O(1) per packet.
const EVICTION_WINDOW: usize = 8;

/// The largest accepted `max_entries`, so the slot count cannot overflow `usize`
/// on 32-bit targets.
pub(crate) const MAX_ENTRIES_LIMIT: usize = 1 << 24;

/// A flow in the originator's direction: `local` opened it towards `remote`.
///
/// For ICMP echo, `local_port` is the echo identifier and `remote_port` is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionKey {
    pub(crate) local: IpAddr,
    pub(crate) remote: IpAddr,
    pub(crate) protocol: u8,
    pub(crate) local_port: u16,
    pub(crate) remote_port: u16,
}

impl ConnectionKey {
    /// FxHash-style multiply-rotate over the key's words: fast, deterministic and
    /// good enough for a bounded table, where a collision costs a probe, not memory.
    fn hash(&self) -> u64 {
        const MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;
        let mix = |state: u64, word: u64| (state ^ word).wrapping_mul(MULTIPLIER).rotate_left(26);
        let mut state = 0;
        for address in [self.local, self.remote] {
            let bits = match address {
                IpAddr::V4(v4) => u128::from(v4.to_bits()),
                IpAddr::V6(v6) => v6.to_bits(),
            };
            state = mix(state, u64::try_from(bits >> 64).unwrap_or(0));
            state = mix(
                state,
                u64::try_from(bits & u128::from(u64::MAX)).unwrap_or(0),
            );
        }
        let rest = u64::from(self.protocol) << 32
            | u64::from(self.local_port) << 16
            | u64::from(self.remote_port);
        state = mix(state, rest);
        state ^ (state >> 32)
    }
}

/// Where a flow is in its lifetime. Each state has its own idle timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// We sent a SYN; nothing came back yet.
    TcpSynSent,
    /// The remote side answered.
    TcpEstablished,
    /// A FIN was seen in either direction.
    TcpClosing,
    /// An RST was seen in either direction.
    TcpClosed,
    /// UDP sent by us, no reply yet.
    UdpUnreplied,
    /// UDP seen in both directions.
    UdpStream,
    /// An ICMP echo request we sent.
    IcmpEcho,
}

impl State {
    /// How long the flow survives without traffic.
    pub(crate) const fn timeout(self) -> Duration {
        match self {
            Self::TcpSynSent => TCP_SYN_SENT,
            Self::TcpEstablished => TCP_ESTABLISHED,
            Self::TcpClosing => TCP_CLOSING,
            Self::TcpClosed => TCP_CLOSE,
            Self::UdpUnreplied => UDP_UNREPLIED,
            Self::UdpStream => UDP_STREAM,
            Self::IcmpEcho => ICMP_ECHO,
        }
    }

    /// Whether a full table may evict this flow to admit a new one.
    ///
    /// Like Linux's `early_drop`, only flows that never became two-way (or are
    /// already closing) are sacrificed; established connections are protected.
    const fn evictable(self) -> bool {
        match self {
            Self::TcpSynSent
            | Self::TcpClosing
            | Self::TcpClosed
            | Self::UdpUnreplied
            | Self::IcmpEcho => true,
            Self::TcpEstablished | Self::UdpStream => false,
        }
    }
}

/// One tracked flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) key: ConnectionKey,
    pub(crate) state: State,
    /// The flow is live while `now < expires_at`.
    pub(crate) expires_at: Duration,
}

impl Entry {
    pub(crate) fn new(key: ConnectionKey, state: State, now: Duration) -> Self {
        Self {
            key,
            state,
            expires_at: now.saturating_add(state.timeout()),
        }
    }

    /// Moves to `state` and restarts its idle timer.
    pub(crate) fn refresh(&mut self, state: State, now: Duration) {
        self.state = state;
        self.expires_at = now.saturating_add(state.timeout());
    }

    const fn is_live(&self, now: Duration) -> bool {
        self.expires_at.as_nanos() > now.as_nanos()
    }
}

/// How an insert made room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Inserted {
    /// A free or stale slot was used.
    Fresh,
    /// A live, evictable flow was dropped to make room.
    Evicted,
}

/// The table is full and nothing near the key's home may be evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Full;

/// Fixed-capacity flow table.
///
/// Invariants: `slots.len()` is a power of two and at least `2 * max_entries`, so at
/// least half the slots are always empty and every probe terminates; `len` counts the
/// occupied slots (stale ones included) and never exceeds `max_entries`.
#[derive(Debug)]
pub(crate) struct Table {
    slots: Box<[Option<Entry>]>,
    max_entries: usize,
    len: usize,
}

impl Table {
    /// Allocates room for `max_entries` flows, clamped to `1..=MAX_ENTRIES_LIMIT`.
    pub(crate) fn new(max_entries: usize) -> Self {
        let max_entries = max_entries.clamp(1, MAX_ENTRIES_LIMIT);
        // Cannot overflow: max_entries <= 2^24.
        let slot_count = max_entries.saturating_mul(2).next_power_of_two();
        Self {
            slots: vec![None; slot_count].into_boxed_slice(),
            max_entries,
            len: 0,
        }
    }

    pub(crate) const fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    const fn mask(&self) -> usize {
        self.slots.len().saturating_sub(1)
    }

    fn home(&self, key: &ConnectionKey) -> usize {
        // Truncating the hash to the slot mask is the intent.
        usize::try_from(key.hash() & u64::try_from(self.mask()).unwrap_or(u64::MAX)).unwrap_or(0)
    }

    const fn next(&self, index: usize) -> usize {
        index.wrapping_add(1) & self.mask()
    }

    /// The live flow for `key`, if any. Stale flows are treated as absent.
    pub(crate) fn get_mut(&mut self, key: &ConnectionKey, now: Duration) -> Option<&mut Entry> {
        let index = self.find(key)?;
        self.slots
            .get_mut(index)?
            .as_mut()
            .filter(|entry| entry.is_live(now))
    }

    /// The slot holding `key`, live or stale.
    fn find(&self, key: &ConnectionKey) -> Option<usize> {
        let mut index = self.home(key);
        for _ in 0..self.slots.len() {
            match self.slots.get(index)? {
                None => return None,
                Some(entry) if entry.key == *key => return Some(index),
                Some(_) => index = self.next(index),
            }
        }
        None
    }

    /// Inserts `entry`, whose key must have no live flow in the table.
    ///
    /// Reuses a stale slot on the key's probe path if there is one. When the table is
    /// full, evicts the soonest-expiring stale or evictable flow within
    /// [`EVICTION_WINDOW`] slots of the key's home, or fails with [`Full`].
    pub(crate) fn insert(&mut self, entry: Entry, now: Duration) -> Result<Inserted, Full> {
        let mut index = self.home(&entry.key);
        for _ in 0..self.slots.len() {
            let Some(slot) = self.slots.get_mut(index) else {
                break;
            };
            match slot {
                // Overwriting an occupied slot keeps every probe path intact.
                Some(old) if old.key == entry.key || !old.is_live(now) => {
                    *old = entry;
                    return Ok(Inserted::Fresh);
                }
                Some(_) => index = self.next(index),
                None if self.len < self.max_entries => {
                    *slot = Some(entry);
                    self.len = self.len.saturating_add(1);
                    return Ok(Inserted::Fresh);
                }
                None => {
                    let (victim, was_live) = self.eviction_victim(&entry.key, now).ok_or(Full)?;
                    self.remove(victim);
                    // There is room now, and no stale slot on the path: plain insert.
                    return self.insert(entry, now).map(|inserted| {
                        if was_live {
                            Inserted::Evicted
                        } else {
                            inserted
                        }
                    });
                }
            }
        }
        Err(Full)
    }

    /// The slot to free for `key`, and whether its flow was still live.
    fn eviction_victim(&self, key: &ConnectionKey, now: Duration) -> Option<(usize, bool)> {
        let mut index = self.home(key);
        let mut victim: Option<(usize, Duration)> = None;
        for _ in 0..EVICTION_WINDOW.min(self.slots.len()) {
            if let Some(Some(entry)) = self.slots.get(index)
                && (entry.state.evictable() || !entry.is_live(now))
                && victim.is_none_or(|(_, soonest)| entry.expires_at < soonest)
            {
                victim = Some((index, entry.expires_at));
            }
            index = self.next(index);
        }
        victim.map(|(index, expires_at)| (index, expires_at > now))
    }

    /// Empties `index` and shifts later members of its cluster back so that no probe
    /// path crosses the hole (Knuth's Algorithm R).
    fn remove(&mut self, mut hole: usize) {
        if self.slots.get_mut(hole).and_then(Option::take).is_none() {
            return;
        }
        self.len = self.len.saturating_sub(1);
        let mut index = self.next(hole);
        for _ in 0..self.slots.len() {
            let Some(Some(entry)) = self.slots.get(index).copied() else {
                return;
            };
            let entry_home = self.home(&entry.key);
            let mask = self.mask();
            // Move the entry back unless its home lies cyclically in (hole, index].
            if index.wrapping_sub(entry_home) & mask >= index.wrapping_sub(hole) & mask {
                if let Some(slot) = self.slots.get_mut(hole) {
                    *slot = Some(entry);
                }
                if let Some(slot) = self.slots.get_mut(index) {
                    *slot = None;
                }
                hole = index;
            }
            index = self.next(index);
        }
    }

    /// Removes every stale flow.
    pub(crate) fn expire(&mut self, now: Duration) {
        // Start just after an empty slot: shifts stay inside a cluster, so every
        // entry moved by `remove` lands on a slot this pass has yet to visit.
        let Some(empty) = self.slots.iter().position(Option::is_none) else {
            return;
        };
        let mut index = empty;
        for _ in 0..self.slots.len() {
            index = self.next(index);
            while self
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|entry| !entry.is_live(now))
            {
                self.remove(index);
            }
        }
    }
}
