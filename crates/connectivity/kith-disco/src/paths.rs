//! Per-peer path selection: which UDP endpoint to send to, or the relay.
//!
//! Modelled on Tailscale's magicsock `endpoint`:
//!
//! - Candidates come from the control plane, from the peer's [`CallMeMaybe`], and from addresses
//!   the peer pinged us from.
//! - While data is flowing ([`PeerPaths::note_activity`]), candidates are pinged. With no trusted
//!   direct path every candidate is pinged every [`DISCO_PING_INTERVAL`]; with one, the best path
//!   gets a heartbeat every [`HEARTBEAT_INTERVAL`] and the others an upgrade probe every
//!   [`UPGRADE_INTERVAL`].
//! - A pong makes its path trusted for [`TRUST_DURATION`]. The best trusted path wins; when none
//!   is trusted, traffic falls back to the relay.
//! - A hole punch is [`PeerPaths::request_hole_punch`]: a [`CallMeMaybe`] over the relay plus an
//!   immediate ping to every candidate, so both sides ping each other at the same time.
//!
//! Time is an input. Nothing here reads a clock.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use kith_crypto::{Context, Hasher, RandomError};
use kith_pmtu::SizeSearch;
use kith_types::NodeId;

use crate::message::{
    CallMeMaybe, MAX_CALL_ME_MAYBE_ENDPOINTS, Message, PING_LEN, Ping, Pong, TX_ID_LEN, TxId,
};
use crate::seal::{MIN_PACKET_LEN, RandomSource};

/// How often the best path is pinged to keep it trusted. Tailscale: `heartbeatInterval`.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
/// How long a path stays trusted after its last pong. Tailscale: `trustUDPAddrDuration`.
/// Just over two heartbeats, so one lost pong does not drop the path.
pub const TRUST_DURATION: Duration = Duration::from_millis(6500);
/// How long to wait for a pong before forgetting the ping. Tailscale: `pingTimeoutDuration`.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// The minimum gap between pings to one candidate. Tailscale: `discoPingInterval`.
pub const DISCO_PING_INTERVAL: Duration = Duration::from_secs(5);
/// How often non-best candidates are probed while a direct path is trusted.
/// Tailscale: `upgradeUDPDirectInterval`.
pub const UPGRADE_INTERVAL: Duration = Duration::from_secs(60);
/// How long after the last data packet pinging continues. Tailscale: `sessionActiveTimeout`.
pub const SESSION_ACTIVE_TIMEOUT: Duration = Duration::from_secs(45);
/// The minimum gap between two call-me-maybes. Matches [`DISCO_PING_INTERVAL`] (no Tailscale
/// equivalent): a punch cannot usefully repeat faster than its pings.
pub const CALL_ME_MAYBE_INTERVAL: Duration = Duration::from_secs(5);

/// The most candidates kept from the control plane.
pub const MAX_CONTROL_CANDIDATES: usize = 32;
/// The most candidates learned from pings we received. The oldest is evicted first.
pub const MAX_OBSERVED_CANDIDATES: usize = 8;
/// The most local endpoints advertised in a call-me-maybe.
pub const MAX_LOCAL_ENDPOINTS: usize = MAX_CALL_ME_MAYBE_ENDPOINTS;
/// The most pings awaiting a pong. Every candidate can have about one in flight.
pub const MAX_IN_FLIGHT_PINGS: usize =
    MAX_CONTROL_CANDIDATES + MAX_CALL_ME_MAYBE_ENDPOINTS + MAX_OBSERVED_CANDIDATES;
/// The most undelivered events; the oldest is dropped first.
pub const MAX_PENDING_EVENTS: usize = 16;

/// What WireGuard adds to an IP packet to make a UDP payload: its 16-byte header and
/// 16-byte tag. A path that carries a UDP payload of `n` bytes carries tunnel packets of
/// `n - 32`.
pub const WIREGUARD_DATA_OVERHEAD: u16 = 32;
/// The smallest UDP payload a probe tries: what carries a tunnel packet of
/// [`kith_pmtu::TUNNEL_FLOOR`], which every working path is taken to carry.
const SIZE_FLOOR: u16 = kith_pmtu::TUNNEL_FLOOR + WIREGUARD_DATA_OVERHEAD;

/// The largest probe sent: a full Ethernet frame's UDP payload over IPv4. Bigger tunnel
/// MTUs are trusted rather than probed, since no internet path is larger.
pub const MAX_PROBE_PAYLOAD: u16 = 1472;
const _: () = assert!(
    TRUST_DURATION.as_millis() < PING_TIMEOUT.saturating_mul(kith_pmtu::ATTEMPTS).as_millis(),
    "a silent path must lose trust before its size probes can all be lost"
);

/// The derivation context for transaction IDs.
pub const TX_ID_CONTEXT: Context = Context::new("kith 2026-10 disco tx id v1");

/// Where a packet to the peer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathTarget {
    /// Straight to this UDP endpoint.
    Direct(SocketAddr),
    /// Through the peer's relay.
    Relay,
}

/// Why [`PeerPaths::best_path`] chose what it did, for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathReason {
    /// A direct endpoint answered pings recently.
    Direct {
        /// The endpoint in use.
        endpoint: SocketAddr,
        /// Its smoothed round-trip time, once measured.
        latency: Option<Duration>,
    },
    /// No direct endpoint answered; `candidates` were tried.
    NoDirectAnswer {
        /// Known direct endpoints.
        candidates: usize,
        /// Whether a relay carries traffic meanwhile.
        relay: bool,
    },
}

impl fmt::Display for PathReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Direct {
                endpoint,
                latency: Some(latency),
            } => write!(f, "direct to {endpoint} ({} ms)", latency.as_millis()),
            Self::Direct { endpoint, .. } => write!(f, "direct to {endpoint}"),
            Self::NoDirectAnswer { candidates, relay } => {
                f.write_str(if relay {
                    "relay: "
                } else {
                    "offline: no relay, and "
                })?;
                match candidates {
                    0 => f.write_str("no direct addresses known for this peer yet"),
                    1 => f.write_str("its one known address did not answer"),
                    n => write!(f, "none of its {n} known addresses answered"),
                }
            }
        }
    }
}

/// A message the driver should seal and send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transmit {
    /// Where to send it.
    pub to: PathTarget,
    /// What to send.
    pub message: Message,
}

/// Something the driver should know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// The best path changed; `None` means the peer is unreachable.
    PathChanged(Option<PathTarget>),
    /// The peer saw our packets come from this address (our reflexive endpoint).
    EndpointObserved(SocketAddr),
    /// The largest tunnel packet the current path carries changed; `None` means no limit
    /// below the tunnel MTU is known, as on the relay.
    PathMtuChanged(Option<u16>),
}

/// Where a candidate came from. A candidate is dropped once it has no source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Sources {
    control: bool,
    call_me_maybe: bool,
    /// When the peer last pinged us from this address.
    observed_at: Option<Duration>,
}

impl Sources {
    const fn is_empty(self) -> bool {
        !self.control && !self.call_me_maybe && self.observed_at.is_none()
    }
}

#[derive(Debug, Clone, Default)]
struct Candidate {
    sources: Sources,
    last_ping: Option<Duration>,
    last_pong: Option<Duration>,
    latency: Option<Duration>,
    /// Ping as soon as the rate limit allows, even when idle: part of a hole punch.
    punch: bool,
}

#[derive(Debug, Clone, Copy)]
struct SentPing {
    to: SocketAddr,
    at: Duration,
    /// The UDP payload size, when this ping is a size probe.
    probe: Option<u16>,
}

/// The size search on the current direct path.
#[derive(Debug, Clone, Default)]
struct PathSize {
    /// The path searched; a new path starts over.
    path: Option<SocketAddr>,
    search: SizeSearch,
    /// What was last reported.
    reported: Option<u16>,
}

/// The path-selection state machine for one peer.
///
/// Drive it with the `handle_*` methods, then drain [`Self::poll_transmit`] and
/// [`Self::poll_event`], and call [`Self::poll_transmit`] again at [`Self::poll_timeout`].
#[cfg_attr(test, derive(Clone))]
pub struct PeerPaths {
    local: NodeId,
    peer: NodeId,
    tx_seed: [u8; 32],
    tx_counter: u64,
    candidates: BTreeMap<SocketAddr, Candidate>,
    in_flight: BTreeMap<TxId, SentPing>,
    local_endpoints: Vec<SocketAddr>,
    relay_available: bool,
    best_direct: Option<SocketAddr>,
    reported: Option<PathTarget>,
    last_activity: Option<Duration>,
    call_me_maybe_due: bool,
    last_call_me_maybe: Option<Duration>,
    events: VecDeque<Event>,
    /// The largest UDP payload worth proving the path carries: the tunnel MTU plus
    /// WireGuard's overhead, at most [`MAX_PROBE_PAYLOAD`]. Zero turns probing off.
    size_target: u16,
    size: PathSize,
    /// The latest time passed in; time never goes backwards from here.
    now: Duration,
}

impl fmt::Debug for PeerPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerPaths")
            .field("peer", &self.peer)
            .field("best", &self.best_path())
            .field("candidates", &self.candidates.len())
            .field("in_flight", &self.in_flight.len())
            .finish_non_exhaustive()
    }
}

impl PeerPaths {
    /// Creates the state for reaching `peer` as `local`. The relay starts out available.
    ///
    /// `random` seeds transaction IDs, so a pong recorded before a restart cannot match a
    /// new ping.
    pub fn new(
        local: NodeId,
        peer: NodeId,
        random: &mut impl RandomSource,
    ) -> Result<Self, RandomError> {
        let mut tx_seed = [0; 32];
        random.fill(&mut tx_seed)?;
        let mut paths = Self {
            local,
            peer,
            tx_seed,
            tx_counter: 0,
            candidates: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            local_endpoints: Vec::new(),
            relay_available: true,
            best_direct: None,
            reported: None,
            last_activity: None,
            call_me_maybe_due: false,
            last_call_me_maybe: None,
            events: VecDeque::new(),
            size_target: 0,
            size: PathSize::default(),
            now: Duration::ZERO,
        };
        paths.report_path();
        Ok(paths)
    }

    /// The path to send data on now: the best trusted direct path, else the relay.
    pub fn best_path(&self) -> Option<PathTarget> {
        self.best_direct
            .map(PathTarget::Direct)
            .or(self.relay_available.then_some(PathTarget::Relay))
    }

    /// Why the current path was chosen.
    pub fn explain(&self) -> PathReason {
        match self.best_direct {
            Some(endpoint) => PathReason::Direct {
                endpoint,
                latency: self.candidates.get(&endpoint).and_then(|c| c.latency),
            },
            None => PathReason::NoDirectAnswer {
                candidates: self.candidates.len(),
                relay: self.relay_available,
            },
        }
    }

    /// Whether the peer's relay can be used.
    pub fn set_relay_available(&mut self, now: Duration, available: bool) {
        self.advance(now);
        self.relay_available = available;
        self.report_path();
    }

    /// Starts finding out whether the direct path carries tunnel packets of `mtu` bytes,
    /// and if not, the largest it does.
    pub fn set_tunnel_mtu(&mut self, mtu: u16) {
        let target = mtu
            .saturating_add(WIREGUARD_DATA_OVERHEAD)
            .min(MAX_PROBE_PAYLOAD);
        if target != self.size_target {
            self.size_target = target;
            self.size.path = None;
            self.size.search = SizeSearch::new(SIZE_FLOOR, target);
            self.report_size();
        }
    }

    /// The largest tunnel packet the current path is known to carry, if smaller than the
    /// tunnel MTU: `None` until a size has been lost, and always on the relay.
    pub fn path_mtu(&self) -> Option<u16> {
        if self.size.path.is_none() || self.size.path != self.best_direct {
            return None;
        }
        let limit = self.size.search.limit()?;
        Some(limit.saturating_sub(WIREGUARD_DATA_OVERHEAD))
    }

    /// Records that data was sent to or received from the peer, which keeps probing alive for
    /// [`SESSION_ACTIVE_TIMEOUT`].
    pub fn note_activity(&mut self, now: Duration) {
        self.advance(now);
        self.last_activity = Some(self.now);
    }

    /// Replaces the peer endpoints known from the control plane. Only the first
    /// [`MAX_CONTROL_CANDIDATES`] are kept.
    pub fn set_candidates(&mut self, now: Duration, endpoints: &[SocketAddr]) {
        self.advance(now);
        for candidate in self.candidates.values_mut() {
            candidate.sources.control = false;
        }
        for endpoint in endpoints.iter().take(MAX_CONTROL_CANDIDATES) {
            self.candidates
                .entry(*endpoint)
                .or_default()
                .sources
                .control = true;
        }
        self.prune();
    }

    /// Replaces our own endpoints, advertised in call-me-maybes. Only the first
    /// [`MAX_LOCAL_ENDPOINTS`] are kept.
    pub fn set_local_endpoints(&mut self, endpoints: &[SocketAddr]) {
        self.local_endpoints.clear();
        self.local_endpoints
            .extend(endpoints.iter().take(MAX_LOCAL_ENDPOINTS));
    }

    /// Starts a hole punch: a call-me-maybe over the relay (at most once per
    /// [`CALL_ME_MAYBE_INTERVAL`]) and a ping to every candidate as soon as its rate limit allows.
    pub fn request_hole_punch(&mut self, now: Duration) {
        self.advance(now);
        if !self.local_endpoints.is_empty() {
            self.call_me_maybe_due = true;
        }
        for candidate in self.candidates.values_mut() {
            candidate.punch = true;
        }
    }

    /// Handles the peer's call-me-maybe: its endpoints replace those of the previous one, and
    /// all of them are pinged as soon as possible, which punches the hole from our side.
    pub fn handle_call_me_maybe(&mut self, now: Duration, call: &CallMeMaybe) {
        self.advance(now);
        for candidate in self.candidates.values_mut() {
            candidate.sources.call_me_maybe = false;
        }
        for endpoint in call.endpoints() {
            let candidate = self.candidates.entry(*endpoint).or_default();
            candidate.sources.call_me_maybe = true;
            // Tailscale also clears the ping rate limit here; we keep it, so a peer that
            // spams call-me-maybes cannot make us flood its endpoints.
            candidate.punch = true;
        }
        self.prune();
    }

    /// Handles a ping that arrived directly from `from`, returning the pong to send back.
    ///
    /// A ping not from this peer is ignored. The source becomes a candidate and is pinged back,
    /// which completes a hole punch the peer started.
    pub fn handle_ping(
        &mut self,
        now: Duration,
        from: SocketAddr,
        ping: &Ping,
    ) -> Option<Transmit> {
        self.advance(now);
        if ping.sender != self.peer {
            return None;
        }
        self.observe(from);
        Some(Transmit {
            to: PathTarget::Direct(from),
            message: Message::Pong(Pong {
                tx_id: ping.tx_id,
                observed: from,
            }),
        })
    }

    /// Handles a pong that arrived directly from `from`. Returns whether it matched a ping.
    ///
    /// Pongs with an unknown transaction ID, or from an address other than the one pinged, are
    /// ignored.
    pub fn handle_pong(&mut self, now: Duration, from: SocketAddr, pong: &Pong) -> bool {
        self.advance(now);
        let Some(sent) = self.in_flight.get(&pong.tx_id).copied() else {
            return false;
        };
        if sent.to != from {
            return false;
        }
        self.in_flight.remove(&pong.tx_id);
        let latency = self.now.saturating_sub(sent.at);
        if let Some(candidate) = self.candidates.get_mut(&from) {
            candidate.last_pong = Some(self.now);
            if sent.probe.is_none() {
                candidate.latency = Some(latency);
            }
        }
        if let Some(size) = sent.probe {
            self.size_answered(from, size);
        }
        self.push_event(Event::EndpointObserved(pong.observed));
        self.choose_best();
        true
    }

    /// Expires pings and trust as of `now`. [`Self::poll_transmit`] calls this itself.
    pub fn handle_timeout(&mut self, now: Duration) {
        self.advance(now);
    }

    /// The next message to send, if any is due at `now`.
    pub fn poll_transmit(&mut self, now: Duration) -> Option<Transmit> {
        self.advance(now);
        if self.call_me_maybe_ready() {
            self.call_me_maybe_due = false;
            self.last_call_me_maybe = Some(self.now);
            let call = CallMeMaybe::new(self.local_endpoints.clone())?;
            return Some(Transmit {
                to: PathTarget::Relay,
                message: Message::CallMeMaybe(call),
            });
        }
        if self.in_flight.len() >= MAX_IN_FLIGHT_PINGS {
            return None;
        }
        if let Some(probe) = self.next_size_probe() {
            return Some(probe);
        }
        let to = self
            .candidates
            .iter()
            .find(|(address, candidate)| {
                self.ping_due_at(**address, candidate)
                    .is_some_and(|due| due <= self.now)
            })
            .map(|(address, _)| *address)?;
        let tx_id = self.next_tx_id();
        self.in_flight.insert(
            tx_id,
            SentPing {
                to,
                at: self.now,
                probe: None,
            },
        );
        if let Some(candidate) = self.candidates.get_mut(&to) {
            candidate.last_ping = Some(self.now);
            candidate.punch = false;
        }
        // As Tailscale does: while relayed, every ping round also asks the peer to ping back,
        // so both NATs open at once. Rate-limited by `CALL_ME_MAYBE_INTERVAL`.
        if self.best_direct.is_none() && self.is_active_at(self.now) {
            self.call_me_maybe_due |= !self.local_endpoints.is_empty();
        }
        Some(Transmit {
            to: PathTarget::Direct(to),
            message: Message::Ping(Ping {
                tx_id,
                sender: self.local,
                padding: 0,
            }),
        })
    }

    /// The next event, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// When [`Self::poll_transmit`] next needs calling: the earliest due ping, call-me-maybe,
    /// ping expiry or trust expiry. A time in the past means now.
    pub fn poll_timeout(&self) -> Option<Duration> {
        let call_me_maybe = self
            .call_me_maybe_due
            .then(|| self.call_me_maybe_allowed_at());
        let pings = if self.in_flight.len() >= MAX_IN_FLIGHT_PINGS {
            None
        } else {
            self.candidates
                .iter()
                .filter_map(|(address, candidate)| self.ping_due_at(*address, candidate))
                .min()
        };
        let expiry = self
            .in_flight
            .values()
            .map(|sent| sent.at.saturating_add(PING_TIMEOUT))
            .min();
        let trust = self
            .best_direct
            .and_then(|address| self.candidates.get(&address))
            .and_then(|candidate| candidate.last_pong)
            .map(|pong| pong.saturating_add(TRUST_DURATION));
        [call_me_maybe, pings, expiry, trust, self.size_due()]
            .into_iter()
            .flatten()
            .min()
    }

    fn advance(&mut self, now: Duration) {
        self.now = self.now.max(now);
        let now = self.now;
        let lost: Vec<(SocketAddr, u16)> = self
            .in_flight
            .values()
            .filter(|sent| sent.at.saturating_add(PING_TIMEOUT) <= now)
            .filter_map(|sent| Some((sent.to, sent.probe?)))
            .collect();
        self.in_flight
            .retain(|_, sent| sent.at.saturating_add(PING_TIMEOUT) > now);
        for (to, size) in lost {
            self.size_lost(to, size);
        }
        if self
            .best_direct
            .is_some_and(|address| !self.is_trusted(address))
        {
            self.choose_best();
        }
    }

    fn is_trusted(&self, address: SocketAddr) -> bool {
        self.candidates
            .get(&address)
            .and_then(|candidate| candidate.last_pong)
            .is_some_and(|pong| pong.saturating_add(TRUST_DURATION) > self.now)
    }

    fn is_active_at(&self, time: Duration) -> bool {
        self.last_activity
            .is_some_and(|activity| activity.saturating_add(SESSION_ACTIVE_TIMEOUT) > time)
    }

    fn call_me_maybe_allowed_at(&self) -> Duration {
        self.last_call_me_maybe.map_or(Duration::ZERO, |last| {
            last.saturating_add(CALL_ME_MAYBE_INTERVAL)
        })
    }

    fn call_me_maybe_ready(&self) -> bool {
        self.call_me_maybe_wanted() && self.call_me_maybe_allowed_at() <= self.now
    }

    fn call_me_maybe_wanted(&self) -> bool {
        self.call_me_maybe_due && self.relay_available
    }

    /// When `address` should next be pinged, or `None` if it should not be.
    fn ping_due_at(&self, address: SocketAddr, candidate: &Candidate) -> Option<Duration> {
        let gap = if candidate.punch {
            DISCO_PING_INTERVAL
        } else if self.best_direct == Some(address) {
            HEARTBEAT_INTERVAL
        } else if self.best_direct.is_some() {
            UPGRADE_INTERVAL
        } else {
            DISCO_PING_INTERVAL
        };
        let due = candidate
            .last_ping
            .map_or(Duration::ZERO, |last| last.saturating_add(gap));
        // A ping due in the past is sent at the next poll, which is never before `self.now`.
        (candidate.punch || self.is_active_at(due.max(self.now))).then_some(due)
    }

    fn observe(&mut self, address: SocketAddr) {
        let now = self.now;
        let candidate = self.candidates.entry(address).or_default();
        if candidate.sources.observed_at.is_none() {
            candidate.punch = true;
        }
        candidate.sources.observed_at = Some(now);
        let observed = self
            .candidates
            .values()
            .filter(|candidate| candidate.sources.observed_at.is_some())
            .count();
        if observed > MAX_OBSERVED_CANDIDATES {
            let oldest = self
                .candidates
                .iter()
                .filter_map(|(address, candidate)| {
                    candidate.sources.observed_at.map(|at| (at, *address))
                })
                .min()
                .map(|(_, address)| address);
            if let Some(candidate) = oldest.and_then(|address| self.candidates.get_mut(&address)) {
                candidate.sources.observed_at = None;
            }
            self.prune();
        }
    }

    fn prune(&mut self) {
        self.candidates
            .retain(|_, candidate| !candidate.sources.is_empty());
        self.in_flight
            .retain(|_, sent| self.candidates.contains_key(&sent.to));
        if self
            .best_direct
            .is_some_and(|address| !self.candidates.contains_key(&address))
        {
            self.choose_best();
        }
    }

    /// Keeps the current path unless a trusted candidate is clearly better; if the current path
    /// is no longer trusted, takes the best trusted candidate.
    fn choose_best(&mut self) {
        let current = self.best_direct.filter(|address| self.is_trusted(*address));
        let mut best = current;
        for (address, candidate) in &self.candidates {
            if !self.is_trusted(*address) {
                continue;
            }
            let Some(latency) = candidate.latency else {
                continue;
            };
            let better = match best.and_then(|b| Some((b, self.candidates.get(&b)?.latency?))) {
                None => true,
                Some((incumbent, incumbent_latency)) => {
                    is_better(*address, latency, incumbent, incumbent_latency)
                }
            };
            if better {
                best = Some(*address);
            }
        }
        self.best_direct = best;
        if best.is_some() {
            // A trusted direct path makes a pending hole punch pointless.
            self.call_me_maybe_due = false;
        }
        self.report_path();
    }

    fn report_path(&mut self) {
        let path = self.best_path();
        if path != self.reported {
            self.reported = path;
            self.push_event(Event::PathChanged(path));
        }
        self.report_size();
    }

    fn report_size(&mut self) {
        let mtu = self.path_mtu();
        if mtu != self.size.reported {
            self.size.reported = mtu;
            self.push_event(Event::PathMtuChanged(mtu));
        }
    }

    /// The current direct path, with the search started over if it is new, when probing
    /// is on and data is flowing.
    fn probed_path(&mut self) -> Option<SocketAddr> {
        let path = self.best_direct?;
        if self.size_target == 0 || !self.is_active_at(self.now) {
            return None;
        }
        if self.size.path != Some(path) {
            self.size.path = Some(path);
            self.size.search = SizeSearch::new(SIZE_FLOOR, self.size_target);
        }
        Some(path)
    }

    /// When a size probe is next due on the current direct path.
    fn size_due(&self) -> Option<Duration> {
        let path = self.best_direct?;
        if self.size_target == 0 || !self.is_active_at(self.now) {
            return None;
        }
        if self.size.path != Some(path) {
            return Some(self.now);
        }
        self.size.search.next_due(self.now)
    }

    fn next_size_probe(&mut self) -> Option<Transmit> {
        let to = self.probed_path()?;
        let size = self.size.search.next_probe(self.now)?;
        let padding = usize::from(size).checked_sub(MIN_PACKET_LEN + PING_LEN)?;
        let tx_id = self.next_tx_id();
        self.in_flight.insert(
            tx_id,
            SentPing {
                to,
                at: self.now,
                probe: Some(size),
            },
        );
        Some(Transmit {
            to: PathTarget::Direct(to),
            message: Message::Ping(Ping {
                tx_id,
                sender: self.local,
                padding: u16::try_from(padding).ok()?,
            }),
        })
    }

    fn size_answered(&mut self, from: SocketAddr, size: u16) {
        if self.size.path == Some(from) {
            self.size.search.answered(size);
            self.report_size();
        }
    }

    /// A probe of `size` got no answer. Losing a path altogether is not mistaken for a
    /// small one: trust lapses after [`TRUST_DURATION`], before a second probe of the same
    /// size can be lost, and a new path starts the search over.
    fn size_lost(&mut self, to: SocketAddr, size: u16) {
        if self.size.path == Some(to) {
            self.size.search.lost(size);
            self.report_size();
        }
    }

    fn push_event(&mut self, event: Event) {
        if self.events.len() >= MAX_PENDING_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn next_tx_id(&mut self) -> TxId {
        let hash = Hasher::new(TX_ID_CONTEXT)
            .update(&self.tx_seed)
            .update(&self.tx_counter.to_le_bytes())
            .finalize();
        self.tx_counter = self.tx_counter.wrapping_add(1);
        let mut tx_id = [0; TX_ID_LEN];
        for (slot, byte) in tx_id.iter_mut().zip(hash.as_bytes()) {
            *slot = *byte;
        }
        tx_id
    }
}

/// Tailscale's `betterAddr`: whether `candidate` should replace `incumbent`.
/// The faster path earns its percentage improvement as points; private addresses earn 20,
/// link-local 30, loopback 50 and IPv6 a further 10. A candidate whose only edge is at most one
/// point (a 1% improvement) never displaces an incumbent: that is the hysteresis.
fn is_better(
    candidate: SocketAddr,
    candidate_latency: Duration,
    incumbent: SocketAddr,
    incumbent_latency: Duration,
) -> bool {
    if candidate == incumbent {
        return false;
    }
    let (mut candidate_points, mut incumbent_points) = (0_u128, 0_u128);
    let (a, b) = (candidate_latency.as_nanos(), incumbent_latency.as_nanos());
    if a > b {
        incumbent_points = improvement_points(b, a);
    } else {
        candidate_points = improvement_points(a, b);
    }
    candidate_points = candidate_points.saturating_add(address_points(candidate.ip()));
    incumbent_points = incumbent_points.saturating_add(address_points(incumbent.ip()));
    if candidate_points <= 1 && incumbent_points == 0 {
        return false;
    }
    candidate_points > incumbent_points
}

/// `100 - 100 * faster / slower`, truncated; 0 when `slower` is 0.
fn improvement_points(faster: u128, slower: u128) -> u128 {
    faster
        .saturating_mul(100)
        .checked_div(slower)
        .map_or(0, |ratio| 100_u128.saturating_sub(ratio))
}

fn address_points(ip: IpAddr) -> u128 {
    let ip = ip.to_canonical();
    let scope: u128 = if ip.is_loopback() {
        50
    } else if is_link_local(ip) {
        30
    } else if is_private(ip) {
        20
    } else {
        0
    };
    let family = if ip.is_ipv6() { 10 } else { 0 };
    scope.saturating_add(family)
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unicast_link_local(),
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => ip.is_unique_local(),
    }
}

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::unchecked_time_subtraction,
    reason = "simulated test time is small and never overflows"
)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::net::SocketAddr;
    use core::time::Duration;

    use kith_crypto::RandomError;
    use kith_types::NodeId;
    use proptest::prelude::*;

    use super::{
        CALL_ME_MAYBE_INTERVAL, DISCO_PING_INTERVAL, Event, HEARTBEAT_INTERVAL, PathTarget,
        PeerPaths, TRUST_DURATION, Transmit,
    };
    use crate::message::{CallMeMaybe, Message, Ping, Pong};
    use crate::seal::RandomSource;

    struct Counter(u8);

    impl RandomSource for Counter {
        fn fill(&mut self, buffer: &mut [u8]) -> Result<(), RandomError> {
            for byte in buffer {
                self.0 = self.0.wrapping_add(1);
                *byte = self.0;
            }
            Ok(())
        }
    }

    const LOCAL: NodeId = NodeId::from_bytes([1; 32]);
    const PEER: NodeId = NodeId::from_bytes([2; 32]);

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    fn paths() -> PeerPaths {
        PeerPaths::new(LOCAL, PEER, &mut Counter(0)).unwrap()
    }

    fn events(paths: &mut PeerPaths) -> Vec<Event> {
        core::iter::from_fn(|| paths.poll_event()).collect()
    }

    /// Drains every due ping at `now`, returning `(destination, tx_id)`.
    fn pings(paths: &mut PeerPaths, now: Duration) -> Vec<(SocketAddr, [u8; 12])> {
        let mut out = Vec::new();
        while let Some(Transmit { to, message }) = paths.poll_transmit(now) {
            match (to, message) {
                (PathTarget::Direct(to), Message::Ping(ping)) => out.push((to, ping.tx_id)),
                other => panic!("unexpected transmit {other:?}"),
            }
        }
        out
    }

    fn pong(paths: &mut PeerPaths, now: Duration, from: SocketAddr, tx_id: [u8; 12]) -> bool {
        let observed = addr("198.51.100.9:1234");
        paths.handle_pong(now, from, &Pong { tx_id, observed })
    }

    /// Answers every due ping to an address in `rtts` after its round-trip time, until `end`.
    fn simulate(
        paths: &mut PeerPaths,
        rtts: &[(SocketAddr, Duration)],
        start: Duration,
        end: Duration,
    ) {
        let mut now = start;
        let mut pending: Vec<(Duration, SocketAddr, [u8; 12])> = Vec::new();
        while now <= end {
            paths.note_activity(now);
            for (to, tx_id) in pings(paths, now) {
                if let Some((_, rtt)) = rtts.iter().find(|(address, _)| *address == to) {
                    pending.push((now + *rtt, to, tx_id));
                }
            }
            pending.retain(|&(at, from, tx_id)| {
                if at <= now {
                    pong(paths, now, from, tx_id);
                }
                at > now
            });
            now += ms(1);
        }
    }

    #[test]
    fn direct_path_replaces_relay_then_falls_back_when_pongs_stop() {
        let mut paths = paths();
        let direct = addr("203.0.113.1:41641");
        let ours = addr("192.0.2.7:41641");
        let call = Transmit {
            to: PathTarget::Relay,
            message: Message::CallMeMaybe(CallMeMaybe::new(vec![ours]).unwrap()),
        };
        assert_eq!(paths.best_path(), Some(PathTarget::Relay));
        assert_eq!(
            paths.explain().to_string(),
            "relay: no direct addresses known for this peer yet"
        );
        paths.set_local_endpoints(&[ours]);
        paths.note_activity(ms(0));
        paths.set_candidates(ms(0), &[direct]);
        let round = DISCO_PING_INTERVAL;
        let mut tx_id = [0; 12];
        for at in [ms(0), round] {
            match paths.poll_transmit(at) {
                Some(Transmit {
                    to,
                    message: Message::Ping(ping),
                }) => {
                    assert_eq!(to, PathTarget::Direct(direct));
                    tx_id = ping.tx_id;
                }
                other => panic!("expected a ping, got {other:?}"),
            }
            if at == ms(0) {
                assert_eq!(paths.poll_transmit(at), Some(call.clone()));
            }
        }
        // A pong makes the direct path trusted, which cancels the second call-me-maybe.
        let answered = round + ms(20);
        assert!(pong(&mut paths, answered, direct, tx_id));
        assert_eq!(paths.poll_transmit(answered), None);
        assert_eq!(paths.best_path(), Some(PathTarget::Direct(direct)));
        assert_eq!(
            paths.explain().to_string(),
            "direct to 203.0.113.1:41641 (20 ms)"
        );
        assert_eq!(
            events(&mut paths),
            [
                Event::PathChanged(Some(PathTarget::Relay)),
                Event::EndpointObserved(addr("198.51.100.9:1234")),
                Event::PathChanged(Some(PathTarget::Direct(direct))),
            ]
        );

        // The peer goes silent: heartbeats go unanswered and trust lapses exactly on time.
        let expiry = answered + TRUST_DURATION;
        assert_eq!(paths.poll_timeout(), Some(round + HEARTBEAT_INTERVAL));
        drop(pings(&mut paths, round + HEARTBEAT_INTERVAL));
        paths.handle_timeout(expiry - ms(1));
        assert_eq!(paths.best_path(), Some(PathTarget::Direct(direct)));
        paths.handle_timeout(expiry);
        assert_eq!(paths.best_path(), Some(PathTarget::Relay));
        assert_eq!(
            paths.explain().to_string(),
            "relay: its one known address did not answer"
        );
        assert_eq!(
            events(&mut paths),
            [Event::PathChanged(Some(PathTarget::Relay))]
        );
    }

    #[test]
    fn faster_path_wins_only_by_more_than_the_hysteresis() {
        let slow = addr("203.0.113.1:1");
        let fast = addr("203.0.113.2:1");
        let mut paths = paths();
        paths.note_activity(ms(0));
        paths.set_candidates(ms(0), &[slow, fast]);
        let sent = pings(&mut paths, ms(0));
        assert!(pong(&mut paths, ms(50), slow, sent[0].1));
        assert!(pong(&mut paths, ms(50) + ms(1) / 2, fast, sent[1].1));
        assert_eq!(paths.best_path(), Some(PathTarget::Direct(slow)));

        // Over time the fast path's probes show it is clearly better, and it takes over.
        simulate(
            &mut paths,
            &[(slow, ms(50)), (fast, ms(10))],
            ms(51),
            ms(70_000),
        );
        assert_eq!(paths.best_path(), Some(PathTarget::Direct(fast)));
    }

    #[test]
    fn ipv6_wins_a_latency_tie() {
        let v4 = addr("203.0.113.1:1");
        let v6 = addr("[2001:db8::1]:1");
        let mut paths = paths();
        paths.note_activity(ms(0));
        paths.set_candidates(ms(0), &[v4, v6]);
        let sent = pings(&mut paths, ms(0));
        assert!(pong(&mut paths, ms(30), v4, sent[0].1));
        assert!(pong(&mut paths, ms(30), v6, sent[1].1));
        assert_eq!(paths.best_path(), Some(PathTarget::Direct(v6)));
    }

    #[test]
    fn call_me_maybe_pings_its_endpoints_even_when_idle() {
        let mut paths = paths();
        let endpoints = vec![addr("203.0.113.1:1"), addr("[2001:db8::1]:2")];
        assert_eq!(pings(&mut paths, ms(0)), []);
        paths.handle_call_me_maybe(ms(0), &CallMeMaybe::new(endpoints.clone()).unwrap());
        let sent: Vec<_> = pings(&mut paths, ms(0))
            .into_iter()
            .map(|(to, _)| to)
            .collect();
        assert_eq!(sent, endpoints);
        assert_eq!(pings(&mut paths, ms(60_000)), []);
    }

    #[test]
    fn hole_punch_sends_call_me_maybe_over_the_relay_at_a_bounded_rate() {
        let mut paths = paths();
        let ours = addr("192.0.2.7:41641");
        paths.set_local_endpoints(&[ours]);
        paths.set_candidates(ms(0), &[addr("203.0.113.1:1")]);
        paths.request_hole_punch(ms(0));
        let call = Transmit {
            to: PathTarget::Relay,
            message: Message::CallMeMaybe(CallMeMaybe::new(vec![ours]).unwrap()),
        };
        assert_eq!(paths.poll_transmit(ms(0)), Some(call.clone()));
        assert_eq!(pings(&mut paths, ms(0)).len(), 1);

        paths.request_hole_punch(ms(1000));
        assert_eq!(paths.poll_timeout(), Some(CALL_ME_MAYBE_INTERVAL));
        assert_eq!(paths.poll_transmit(CALL_ME_MAYBE_INTERVAL - ms(1)), None);
        assert_eq!(paths.poll_transmit(CALL_ME_MAYBE_INTERVAL), Some(call));
    }

    #[test]
    fn pong_is_ignored_unless_its_tx_id_and_source_match_a_ping() {
        let direct = addr("203.0.113.1:1");
        let mut paths = paths();
        paths.note_activity(ms(0));
        paths.set_candidates(ms(0), &[direct, addr("203.0.113.2:1")]);
        let sent = pings(&mut paths, ms(0));
        assert!(!pong(&mut paths, ms(5), direct, [0xee; 12]));
        assert!(!pong(&mut paths, ms(5), direct, sent[1].1));
        assert_eq!(paths.best_path(), Some(PathTarget::Relay));
        assert!(pong(&mut paths, ms(5), direct, sent[0].1));
        assert!(!pong(&mut paths, ms(6), direct, sent[0].1), "replay");
    }

    #[test]
    fn ping_from_the_peer_is_answered_and_its_source_probed() {
        let from = addr("203.0.113.5:999");
        let mut paths = paths();
        let ping = |sender| Ping {
            tx_id: [4; 12],
            sender,
            padding: 0,
        };
        assert_eq!(paths.handle_ping(ms(0), from, &ping(LOCAL)), None);
        assert_eq!(
            paths.handle_ping(ms(0), from, &ping(PEER)),
            Some(Transmit {
                to: PathTarget::Direct(from),
                message: Message::Pong(Pong {
                    tx_id: [4; 12],
                    observed: from
                }),
            })
        );
        assert_eq!(pings(&mut paths, ms(0)).len(), 1);
    }

    #[test]
    fn pings_respect_rate_limits_and_stop_when_idle() {
        let reachable = addr("203.0.113.1:1");
        let dead = addr("203.0.113.2:1");
        let mut paths = paths();
        paths.set_candidates(ms(0), &[reachable, dead]);
        paths.note_activity(ms(0));
        let mut log: Vec<(Duration, SocketAddr)> = Vec::new();
        let mut now = ms(0);
        while now <= ms(120_000) {
            for (to, tx_id) in pings(&mut paths, now) {
                log.push((now, to));
                if to == reachable {
                    pong(&mut paths, now, to, tx_id);
                }
            }
            // Spam hole-punch requests from the peer; they must not bypass rate limits.
            paths.handle_call_me_maybe(now, &CallMeMaybe::new(vec![dead]).unwrap());
            now += ms(100);
        }
        for target in [reachable, dead] {
            let times: Vec<_> = log
                .iter()
                .filter(|(_, to)| *to == target)
                .map(|(at, _)| *at)
                .collect();
            assert!(
                times
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] >= HEARTBEAT_INTERVAL)
            );
        }
        let dead_times: Vec<_> = log
            .iter()
            .filter(|(_, to)| *to == dead)
            .map(|(at, _)| *at)
            .collect();
        assert!(
            dead_times
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= DISCO_PING_INTERVAL)
        );
        // Activity stopped at 0 s, so the heartbeat stops after the session timeout.
        let last = log
            .iter()
            .filter(|(_, to)| *to == reachable)
            .map(|(at, _)| *at)
            .max()
            .unwrap();
        assert!(last < super::SESSION_ACTIVE_TIMEOUT);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Advance(u16),
        Activity,
        Candidates(Vec<u8>),
        CallMeMaybe(Vec<u8>),
        Punch,
        Ping(u8),
        PongLatest(u8),
        Transmit,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            any::<u16>().prop_map(Op::Advance),
            Just(Op::Activity),
            proptest::collection::vec(0_u8..6, 0..4).prop_map(Op::Candidates),
            proptest::collection::vec(0_u8..6, 0..4).prop_map(Op::CallMeMaybe),
            Just(Op::Punch),
            (0_u8..6).prop_map(Op::Ping),
            (0_u8..6).prop_map(Op::PongLatest),
            Just(Op::Transmit),
        ]
    }

    fn endpoint(index: u8) -> SocketAddr {
        SocketAddr::new([203, 0, 113, index].into(), 41641)
    }

    proptest! {
        /// Nothing happens before `poll_timeout`, and something happens at it.
        #[test]
        fn poll_timeout_matches_the_next_action(ops in proptest::collection::vec(op(), 1..60)) {
            let mut paths = paths();
            paths.set_local_endpoints(&[addr("192.0.2.1:1")]);
            let mut now = ms(0);
            let mut sent: Vec<(SocketAddr, [u8; 12])> = Vec::new();
            for op in ops {
                match op {
                    Op::Advance(millis) => now += ms(millis.into()),
                    Op::Activity => paths.note_activity(now),
                    Op::Candidates(list) => {
                        let list: Vec<_> = list.into_iter().map(endpoint).collect();
                        paths.set_candidates(now, &list);
                    }
                    Op::CallMeMaybe(list) => {
                        let list = list.into_iter().map(endpoint).collect();
                        paths.handle_call_me_maybe(now, &CallMeMaybe::new(list).unwrap());
                    }
                    Op::Punch => paths.request_hole_punch(now),
                    Op::Ping(index) => {
                        drop(paths.handle_ping(now, endpoint(index), &Ping { tx_id: [9; 12], sender: PEER, padding: 0 }));
                    }
                    Op::PongLatest(index) => {
                        if let Some((to, tx_id)) = sent.iter().rev().find(|(to, _)| *to == endpoint(index)) {
                            pong(&mut paths, now, *to, *tx_id);
                        }
                    }
                    Op::Transmit => {
                        while let Some(transmit) = paths.poll_transmit(now) {
                            if let (PathTarget::Direct(to), Message::Ping(ping)) = (transmit.to, transmit.message) {
                                sent.push((to, ping.tx_id));
                            }
                        }
                    }
                }
                drop(events(&mut paths));
                let Some(timeout) = paths.poll_timeout() else {
                    // Nothing is scheduled, so nothing may be due far in the future either.
                    prop_assert_eq!(paths.clone().poll_transmit(now + ms(1_000_000)), None);
                    continue;
                };
                if timeout > now {
                    let mut early = paths.clone();
                    prop_assert_eq!(early.poll_transmit(timeout - Duration::from_nanos(1)), None);
                    prop_assert_eq!(early.poll_event(), None);
                }
                let at = timeout.max(now);
                let mut due = paths.clone();
                let acted = due.poll_transmit(at).is_some() || due.poll_event().is_some();
                prop_assert!(acted || due.poll_timeout().is_none_or(|next| next > at));
            }
        }
    }

    /// Runs `paths` against a path that carries UDP payloads up to `carries` bytes and
    /// loses one probe in `lose_every` (0: none), for `seconds`, answering what arrives.
    fn run_path(
        paths: &mut PeerPaths,
        (carries, lose_every): (u16, u32),
        from: u64,
        seconds: u64,
    ) -> Vec<Event> {
        let to = addr("198.51.100.7:41641");
        let mut probes_sent = 0;
        let mut seen = Vec::new();
        for tick in from * 10..(from + seconds) * 10 {
            let now = ms(tick * 100);
            paths.note_activity(now);
            while let Some(Transmit { message, .. }) = paths.poll_transmit(now) {
                let Message::Ping(ping) = message else {
                    continue;
                };
                let size = crate::seal::MIN_PACKET_LEN
                    + crate::message::PING_LEN
                    + usize::from(ping.padding);
                if ping.padding > 0 {
                    probes_sent += 1;
                    if lose_every > 0 && probes_sent % lose_every == 0 {
                        continue;
                    }
                }
                if size <= usize::from(carries) {
                    let answer = Pong {
                        tx_id: ping.tx_id,
                        observed: addr("192.0.2.1:5"),
                    };
                    paths.handle_pong(now + ms(20), to, &answer);
                }
            }
            seen.extend(events(paths));
        }
        seen
    }

    /// Every path MTU reported, in order.
    fn mtus(events: &[Event]) -> Vec<Option<u16>> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::PathMtuChanged(mtu) => Some(*mtu),
                Event::PathChanged(_) | Event::EndpointObserved(_) => None,
            })
            .collect()
    }

    fn trusted_paths(tunnel_mtu: u16) -> PeerPaths {
        let mut paths = paths();
        paths.set_candidates(ms(0), &[addr("198.51.100.7:41641")]);
        paths.set_tunnel_mtu(tunnel_mtu);
        paths
    }

    #[test]
    fn a_small_path_is_measured_to_within_the_precision() {
        for carries in [1248_u16, 1300, 1240] {
            let mut paths = trusted_paths(1280);
            let seen = run_path(&mut paths, (carries, 0), 0, 120);
            let mtu = paths.path_mtu().unwrap_or(u16::MAX);
            let real = carries - super::WIREGUARD_DATA_OVERHEAD;
            assert!(
                mtu <= real && real - mtu <= kith_pmtu::PRECISION,
                "carries {carries}: {mtu}"
            );
            assert_eq!(mtus(&seen).last(), Some(&Some(mtu)), "carries {carries}");
        }
    }

    #[test]
    fn a_path_that_seems_smaller_than_any_real_path_is_given_no_limit() {
        for carries in [1000_u16, 600, 0] {
            let mut paths = trusted_paths(1280);
            let seen = run_path(&mut paths, (carries, 0), 0, 120);
            assert_eq!(paths.path_mtu(), None, "carries {carries}");
            assert!(mtus(&seen).iter().all(Option::is_none), "carries {carries}");
        }
    }

    #[test]
    fn a_path_that_carries_the_tunnel_mtu_has_no_limit() {
        let mut paths = trusted_paths(1280);
        run_path(&mut paths, (1500, 0), 0, 60);
        assert_eq!(paths.path_mtu(), None);
        assert!(
            !events(&mut paths)
                .iter()
                .any(|e| matches!(e, Event::PathMtuChanged(Some(_))))
        );
    }

    #[test]
    fn one_lost_probe_does_not_shrink_a_path() {
        let mut paths = trusted_paths(1280);
        drop(run_path(&mut paths, (1500, 2), 0, 60));
        assert_eq!(
            paths.path_mtu(),
            None,
            "every other probe lost once is not a limit"
        );
    }

    #[test]
    fn a_path_that_stops_answering_is_not_mistaken_for_a_small_one() {
        let mut paths = trusted_paths(1280);
        let seen: Vec<Event> = [((1500, 0), 0, 5), ((0, 0), 5, 4), ((1500, 0), 9, 30)]
            .into_iter()
            .flat_map(|(path, from, seconds)| run_path(&mut paths, path, from, seconds))
            .collect();
        assert_eq!(paths.path_mtu(), None);
        assert!(
            !seen
                .iter()
                .any(|e| matches!(e, Event::PathMtuChanged(Some(_)))),
            "a brief outage shrank the path: {seen:?}"
        );
    }

    #[test]
    fn a_shrunken_path_is_checked_again_and_grows_back() {
        let mut paths = trusted_paths(1280);
        drop(run_path(&mut paths, (1248, 0), 0, 120));
        assert!(paths.path_mtu().is_some());
        let later = kith_pmtu::RECHECK_INTERVAL.as_secs() + 120;
        let mut grown = paths.clone();
        for tick in 0..later * 10 {
            let now = ms(120_000 + tick * 100);
            grown.note_activity(now);
            while let Some(Transmit { message, .. }) = grown.poll_transmit(now) {
                if let Message::Ping(ping) = message {
                    let pong = Pong {
                        tx_id: ping.tx_id,
                        observed: addr("192.0.2.1:5"),
                    };
                    grown.handle_pong(now + ms(20), addr("198.51.100.7:41641"), &pong);
                }
            }
        }
        assert_eq!(grown.path_mtu(), None, "the full size works again");
    }
}
