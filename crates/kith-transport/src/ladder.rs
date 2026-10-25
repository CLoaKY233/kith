//! The ladder scheduler: which rungs to try, when, and which link to keep.
//!
//! # Why it works this way
//!
//! Blocking rarely announces itself. A censored UDP port usually drops
//! packets silently, so waiting for a cheap rung to time out before trying the
//! next one would cost seconds on every connect. Instead the ladder races
//! rungs happy-eyeballs style (RFC 8305): it starts the cheapest rung, and
//! every [`STAGGER`] that passes without a working link it starts the next
//! one. The first link to come up wins and racing stops. A failure report
//! starts the next rung at once instead of waiting out the stagger.
//!
//! A cheaper link that comes up after a costlier one is still preferred: the
//! ladder switches to it and closes the costlier link after
//! [`SUPERSEDE_GRACE`], so in-flight packets drain and there is a fallback if
//! the newcomer dies immediately.
//!
//! Failed rungs back off exponentially from [`BACKOFF_BASE`] to
//! [`BACKOFF_CAP`]. When every rung of a round has failed, the round
//! restarts from the beginning once a backoff expires. Backoff is what keeps
//! the action rate bounded on hostile networks.
//!
//! Networks are remembered by [`NetworkFingerprint`]. On a known network the
//! race begins at the rung that won last time, and while a link is up the
//! ladder probes one cheaper rung every [`UPGRADE_PROBE_INTERVAL`], so a
//! network that later unblocks UDP gets the fast path back. Roaming to another
//! network starts over.
//!
//! The scheduler is deterministic: no randomness, no clock. Every collection
//! is bounded: per-kind state is a fixed array, the close queue holds only
//! links the driver reported up and still holds open (each is queued at most
//! once and leaves the queue when closed or reported down), and the network
//! cache holds at most
//! [`LadderConfig::cache_capacity`] entries.

use alloc::collections::VecDeque;
use core::time::Duration;

use crate::{
    AttemptId, FailureReason, LinkEvent, LinkId, NetworkFingerprint, TransportKind, TransportSet,
    fingerprint::NetworkCache,
};

/// Delay between starting one rung and the next while nothing is up. RFC 8305
/// recommends 250 ms for connection attempts.
pub const STAGGER: Duration = Duration::from_millis(250);

/// How long a superseded, costlier link stays open after a cheaper one wins.
pub const SUPERSEDE_GRACE: Duration = Duration::from_secs(5);

/// Backoff after the first failure of a rung. It doubles with each further failure.
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);

/// Upper bound on the backoff of a rung.
pub const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// Interval between attempts to upgrade to a cheaper rung while a link is up.
pub const UPGRADE_PROBE_INTERVAL: Duration = Duration::from_secs(60);

/// Default number of networks remembered.
pub const CACHE_CAPACITY: usize = 32;

/// Largest accepted [`LadderConfig::cache_capacity`].
pub const MAX_CACHE_CAPACITY: usize = 1024;

/// How long a remembered winner stays valid. Censorship changes slowly, but it does change.
pub const CACHE_TTL: Duration = Duration::from_hours(24);

/// Tuning knobs. [`LadderConfig::default`] uses the module constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LadderConfig {
    /// Kinds the ladder may start. Must not be empty.
    pub enabled: TransportSet,
    /// See [`STAGGER`].
    pub stagger: Duration,
    /// See [`SUPERSEDE_GRACE`].
    pub supersede_grace: Duration,
    /// See [`BACKOFF_BASE`]. Must not be zero.
    pub backoff_base: Duration,
    /// See [`BACKOFF_CAP`]. Must not be below `backoff_base`.
    pub backoff_cap: Duration,
    /// See [`UPGRADE_PROBE_INTERVAL`]. Must not be zero.
    pub upgrade_probe_interval: Duration,
    /// See [`CACHE_CAPACITY`]. At most [`MAX_CACHE_CAPACITY`]; zero disables the cache.
    pub cache_capacity: usize,
    /// See [`CACHE_TTL`].
    pub cache_ttl: Duration,
}

impl Default for LadderConfig {
    fn default() -> Self {
        Self {
            enabled: TransportSet::default(),
            stagger: STAGGER,
            supersede_grace: SUPERSEDE_GRACE,
            backoff_base: BACKOFF_BASE,
            backoff_cap: BACKOFF_CAP,
            upgrade_probe_interval: UPGRADE_PROBE_INTERVAL,
            cache_capacity: CACHE_CAPACITY,
            cache_ttl: CACHE_TTL,
        }
    }
}

/// A [`LadderConfig`] that would let the ladder misbehave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// No transport kind is enabled, so nothing could ever connect.
    #[error("no transport kind is enabled")]
    NoTransports,
    /// A zero backoff would let failing rungs restart without bound.
    #[error("backoff base must be positive")]
    ZeroBackoff,
    /// The backoff cap is below the base.
    #[error("backoff cap is below the backoff base")]
    BackoffCapBelowBase,
    /// A zero probe interval would re-probe without bound.
    #[error("upgrade probe interval must be positive")]
    ZeroProbeInterval,
    /// The cache capacity exceeds [`MAX_CACHE_CAPACITY`].
    #[error("cache capacity exceeds {MAX_CACHE_CAPACITY}")]
    CacheTooLarge,
}

impl LadderConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.enabled.is_empty() {
            Err(ConfigError::NoTransports)
        } else if self.backoff_base.is_zero() {
            Err(ConfigError::ZeroBackoff)
        } else if self.backoff_cap < self.backoff_base {
            Err(ConfigError::BackoffCapBelowBase)
        } else if self.upgrade_probe_interval.is_zero() {
            Err(ConfigError::ZeroProbeInterval)
        } else if self.cache_capacity > MAX_CACHE_CAPACITY {
            Err(ConfigError::CacheTooLarge)
        } else {
            Ok(())
        }
    }
}

/// What the driver should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Start an attempt. Answer later with [`LinkEvent::Up`] or
    /// [`LinkEvent::Failed`] carrying the same `attempt`.
    Start {
        /// Which rung to try.
        kind: TransportKind,
        /// Identifier to echo in the answer.
        attempt: AttemptId,
    },
    /// Close this link. The ladder has already forgotten it; no `Down` is expected.
    Close(LinkId),
}

/// The externally visible state of one rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RungState {
    /// Nothing in flight.
    Idle,
    /// An attempt was started and has not reported back.
    Connecting,
    /// A link of this kind is up. It is the active link or a superseded one in its grace period.
    Up(LinkId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting(AttemptId),
    Up {
        link: LinkId,
        rtt: Duration,
        /// When a cheaper link replaced this one as active.
        superseded_since: Option<Duration>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rung {
    phase: Phase,
    /// Consecutive failures since the last success.
    failures: u32,
    /// Earliest time the rung may be started again.
    ready_at: Duration,
}

impl Rung {
    const FRESH: Self = Self {
        phase: Phase::Idle,
        failures: 0,
        ready_at: Duration::ZERO,
    };
}

/// One rung per kind, addressed by kind without indexing.
#[derive(Debug, Clone, Copy)]
struct Rungs([Rung; TransportKind::COUNT]);

impl Rungs {
    fn get(&self, kind: TransportKind) -> &Rung {
        let [udp, obfuscated, quic, websocket, reality] = &self.0;
        match kind {
            TransportKind::Udp => udp,
            TransportKind::UdpObfuscated => obfuscated,
            TransportKind::Quic => quic,
            TransportKind::WebSocket => websocket,
            TransportKind::Reality => reality,
        }
    }

    fn get_mut(&mut self, kind: TransportKind) -> &mut Rung {
        let [udp, obfuscated, quic, websocket, reality] = &mut self.0;
        match kind {
            TransportKind::Udp => udp,
            TransportKind::UdpObfuscated => obfuscated,
            TransportKind::Quic => quic,
            TransportKind::WebSocket => websocket,
            TransportKind::Reality => reality,
        }
    }

    fn kind_of(&self, link: LinkId) -> Option<TransportKind> {
        TransportKind::ALL.into_iter().find(
            |kind| matches!(self.get(*kind).phase, Phase::Up { link: held, .. } if held == link),
        )
    }
}

/// A start the ladder intends to make.
#[derive(Debug, Clone, Copy)]
struct Plan {
    due: Duration,
    kind: TransportKind,
    /// Every rung of the round has failed; this start begins a new round.
    restart: bool,
}

/// Sans-IO transport scheduler for one peer or relay target.
///
/// Drive it with [`Ladder::handle_event`] and [`Ladder::set_fingerprint`];
/// after each call, and whenever [`Ladder::poll_timeout`] passes, drain
/// [`Ladder::poll_action`] until it returns `None`. Time is any monotonic
/// clock the caller likes, as a [`Duration`] since an arbitrary origin.
#[derive(Debug, Clone)]
pub struct Ladder {
    config: LadderConfig,
    rungs: Rungs,
    active: Option<(LinkId, TransportKind)>,
    /// Where the current race begins: the cached winner or the cheapest enabled kind.
    origin: TransportKind,
    /// Kinds started in the current race round.
    tried: TransportSet,
    /// Earliest time the race may start another rung.
    next_start_at: Duration,
    /// Earliest time of the next upgrade probe while a link is up.
    next_probe_at: Duration,
    fingerprint: Option<NetworkFingerprint>,
    cache: NetworkCache,
    closes: VecDeque<LinkId>,
    /// Latest time seen, used as the deadline for queued work.
    now: Duration,
    restarts: u64,
    /// Identifier of the next attempt.
    next_attempt: u64,
}

impl Ladder {
    /// A ladder that will start its first rung on the first poll.
    pub fn new(config: LadderConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let origin = config.enabled.cheapest().ok_or(ConfigError::NoTransports)?;
        Ok(Self {
            config,
            rungs: Rungs([Rung::FRESH; TransportKind::COUNT]),
            active: None,
            origin,
            tried: TransportSet::EMPTY,
            next_start_at: Duration::ZERO,
            next_probe_at: Duration::ZERO,
            fingerprint: None,
            cache: NetworkCache::new(config.cache_capacity, config.cache_ttl),
            closes: VecDeque::new(),
            next_attempt: 0,
            now: Duration::ZERO,
            restarts: 0,
        })
    }

    /// The link traffic should use, and its kind.
    pub fn active_link(&self) -> Option<(LinkId, TransportKind)> {
        self.active
    }

    /// Handshake round-trip time of the active link.
    pub fn active_rtt(&self) -> Option<Duration> {
        let (_, kind) = self.active?;
        match self.rungs.get(kind).phase {
            Phase::Up { rtt, .. } => Some(rtt),
            Phase::Idle | Phase::Connecting(_) => None,
        }
    }

    /// What the ladder believes about `kind`.
    pub fn rung_state(&self, kind: TransportKind) -> RungState {
        match self.rungs.get(kind).phase {
            Phase::Idle => RungState::Idle,
            Phase::Connecting(_) => RungState::Connecting,
            Phase::Up { link, .. } => RungState::Up(link),
        }
    }

    /// Kinds the ladder may start.
    pub fn enabled(&self) -> TransportSet {
        self.config.enabled
    }

    /// The network the ladder believes it is on.
    pub fn fingerprint(&self) -> Option<NetworkFingerprint> {
        self.fingerprint
    }

    /// How many times every rung of a round failed and the race began again.
    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// Reports the current network. A change means roaming: every link is
    /// closed and the race starts over, at the rung remembered for the new
    /// network if there is a fresh one.
    pub fn set_fingerprint(&mut self, now: Duration, fingerprint: NetworkFingerprint) {
        self.observe(now);
        if self.fingerprint == Some(fingerprint) {
            return;
        }
        self.fingerprint = Some(fingerprint);
        for kind in TransportKind::ALL {
            if let Phase::Up { link, .. } = self.rungs.get(kind).phase {
                self.queue_close(link);
            }
            *self.rungs.get_mut(kind) = Rung::FRESH;
        }
        self.active = None;
        let enabled = self.config.enabled;
        self.origin = self
            .cache
            .lookup(now, fingerprint)
            .filter(|kind| enabled.contains(*kind))
            .or_else(|| enabled.cheapest())
            .unwrap_or(self.origin);
        self.begin_race(now);
    }

    /// Changes which kinds may be started. Links and attempts of kinds no
    /// longer enabled are dropped; their links are closed.
    pub fn set_enabled(&mut self, now: Duration, enabled: TransportSet) -> Result<(), ConfigError> {
        let cheapest = enabled.cheapest().ok_or(ConfigError::NoTransports)?;
        self.observe(now);
        self.config.enabled = enabled;
        if !enabled.contains(self.origin) {
            self.origin = cheapest;
        }
        for kind in TransportKind::ALL
            .into_iter()
            .filter(|kind| !enabled.contains(*kind))
        {
            let rung = self.rungs.get_mut(kind);
            let phase = rung.phase;
            rung.phase = Phase::Idle;
            if let Phase::Up { link, .. } = phase {
                self.queue_close(link);
                self.forget_link(now, link);
            }
        }
        Ok(())
    }

    /// Feeds a driver observation.
    pub fn handle_event(&mut self, now: Duration, event: LinkEvent) {
        self.observe(now);
        match event {
            LinkEvent::Up { link, attempt, rtt } => self.on_up(now, link, attempt, rtt),
            LinkEvent::Down { link, reason } => self.on_down(now, link, reason),
            LinkEvent::Failed { attempt, reason } => self.on_failed(now, attempt, reason),
        }
    }

    /// The next thing the driver should do at `now`, if anything is due.
    pub fn poll_action(&mut self, now: Duration) -> Option<Action> {
        self.observe(now);
        if let Some(link) = self.closes.pop_front() {
            return Some(Action::Close(link));
        }
        if let Some((kind, link)) = self.expired_superseded(now) {
            self.rungs.get_mut(kind).phase = Phase::Idle;
            return Some(Action::Close(link));
        }
        let plan = self.plan().filter(|plan| plan.due <= now)?;
        if plan.restart {
            self.tried = TransportSet::EMPTY;
            self.restarts = self.restarts.saturating_add(1);
        }
        self.tried = self.tried.with(plan.kind);
        let attempt = AttemptId(self.next_attempt);
        self.next_attempt = self.next_attempt.saturating_add(1);
        self.rungs.get_mut(plan.kind).phase = Phase::Connecting(attempt);
        if self.active.is_some() {
            self.next_probe_at = now.saturating_add(self.config.upgrade_probe_interval);
        } else {
            self.next_start_at = now.saturating_add(self.config.stagger);
        }
        Some(Action::Start {
            kind: plan.kind,
            attempt,
        })
    }

    /// When [`Ladder::poll_action`] will next have something to do. `None`
    /// means only an event can create work. A time in the past means now.
    pub fn poll_timeout(&self) -> Option<Duration> {
        if !self.closes.is_empty() {
            return Some(self.now);
        }
        let grace_deadline = self
            .superseded()
            .map(|(_, _, since)| self.grace_end(since))
            .min();
        let start_deadline = self.plan().map(|plan| plan.due);
        match (grace_deadline, start_deadline) {
            (Some(grace), Some(start)) => Some(grace.min(start)),
            (deadline, None) | (None, deadline) => deadline,
        }
    }

    fn observe(&mut self, now: Duration) {
        self.now = self.now.max(now);
    }

    fn begin_race(&mut self, now: Duration) {
        self.tried = TransportSet::EMPTY;
        self.next_start_at = now;
    }

    fn queue_close(&mut self, link: LinkId) {
        if !self.closes.contains(&link) {
            self.closes.push_back(link);
        }
    }

    /// The kind whose rung is waiting for `attempt`. Abandoned attempts have none.
    fn waiting_for(&self, attempt: AttemptId) -> Option<TransportKind> {
        TransportKind::ALL
            .into_iter()
            .find(|kind| self.rungs.get(*kind).phase == Phase::Connecting(attempt))
    }

    fn on_up(&mut self, now: Duration, link: LinkId, attempt: AttemptId, rtt: Duration) {
        let kind = self.waiting_for(attempt);
        let Some(kind) = kind.filter(|_| self.rungs.kind_of(link).is_none()) else {
            // Late or stray: nobody is waiting for this link.
            self.queue_close(link);
            return;
        };
        match self.active {
            Some((_, active_kind)) if active_kind < kind => {
                // A costlier racer finished after the winner. Not needed.
                let rung = self.rungs.get_mut(kind);
                rung.phase = Phase::Idle;
                rung.failures = 0;
                self.queue_close(link);
            }
            active => {
                if let Some((_, previous)) = active
                    && let Phase::Up {
                        superseded_since, ..
                    } = &mut self.rungs.get_mut(previous).phase
                {
                    *superseded_since = Some(now);
                }
                let rung = self.rungs.get_mut(kind);
                rung.phase = Phase::Up {
                    link,
                    rtt,
                    superseded_since: None,
                };
                rung.failures = 0;
                rung.ready_at = now;
                self.activate(now, link, kind);
            }
        }
    }

    fn on_down(&mut self, now: Duration, link: LinkId, _reason: FailureReason) {
        // A link waiting to be closed is already forgotten; just stop waiting.
        self.closes.retain(|queued| *queued != link);
        let Some(kind) = self.rungs.kind_of(link) else {
            return;
        };
        self.rungs.get_mut(kind).phase = Phase::Idle;
        self.record_failure(now, kind);
        self.forget_link(now, link);
    }

    fn on_failed(&mut self, now: Duration, attempt: AttemptId, _reason: FailureReason) {
        let Some(kind) = self.waiting_for(attempt) else {
            return;
        };
        self.rungs.get_mut(kind).phase = Phase::Idle;
        self.record_failure(now, kind);
        if self.active.is_none() {
            // Happy eyeballs: a definite failure need not wait out the stagger.
            self.next_start_at = self.next_start_at.min(now);
        }
    }

    /// Handles a link that is gone. If it was active, falls back to the
    /// cheapest remaining link, or races again if there is none.
    fn forget_link(&mut self, now: Duration, link: LinkId) {
        if self.active.map(|(active, _)| active) != Some(link) {
            return;
        }
        self.active = None;
        let fallback = TransportKind::ALL.into_iter().find_map(|kind| {
            match &mut self.rungs.get_mut(kind).phase {
                Phase::Up {
                    link,
                    superseded_since,
                    ..
                } => {
                    *superseded_since = None;
                    Some((*link, kind))
                }
                Phase::Idle | Phase::Connecting(_) => None,
            }
        });
        match fallback {
            Some((link, kind)) => self.activate(now, link, kind),
            None => self.begin_race(now),
        }
    }

    fn activate(&mut self, now: Duration, link: LinkId, kind: TransportKind) {
        self.active = Some((link, kind));
        self.next_probe_at = now.saturating_add(self.config.upgrade_probe_interval);
        if let Some(fingerprint) = self.fingerprint {
            self.cache.store(now, fingerprint, kind);
        }
    }

    fn record_failure(&mut self, now: Duration, kind: TransportKind) {
        let base = self.config.backoff_base;
        let cap = self.config.backoff_cap;
        let rung = self.rungs.get_mut(kind);
        rung.failures = rung.failures.saturating_add(1);
        let delay = 2_u32
            .checked_pow(rung.failures.saturating_sub(1))
            .and_then(|factor| base.checked_mul(factor))
            .map_or(cap, |delay| delay.min(cap));
        rung.ready_at = now.saturating_add(delay);
    }

    fn grace_end(&self, since: Duration) -> Duration {
        since.saturating_add(self.config.supersede_grace)
    }

    fn superseded(&self) -> impl Iterator<Item = (TransportKind, LinkId, Duration)> + '_ {
        TransportKind::ALL
            .into_iter()
            .filter_map(|kind| match self.rungs.get(kind).phase {
                Phase::Up {
                    link,
                    superseded_since: Some(since),
                    ..
                } => Some((kind, link, since)),
                Phase::Up { .. } | Phase::Idle | Phase::Connecting(_) => None,
            })
    }

    fn expired_superseded(&self, now: Duration) -> Option<(TransportKind, LinkId)> {
        self.superseded()
            .find(|(_, _, since)| self.grace_end(*since) <= now)
            .map(|(kind, link, _)| (kind, link))
    }

    /// The next start the ladder would make, and when. Shared by
    /// [`Self::poll_action`] and [`Self::poll_timeout`] so they cannot disagree.
    fn plan(&self) -> Option<Plan> {
        let idle = |kind: &TransportKind| {
            self.config.enabled.contains(*kind) && self.rungs.get(*kind).phase == Phase::Idle
        };
        let earliest = |not_before: Duration, kinds: &mut dyn Iterator<Item = TransportKind>| {
            kinds
                .map(|kind| (not_before.max(self.rungs.get(kind).ready_at), kind))
                .min_by_key(|(due, _)| *due)
        };

        if let Some((_, active_kind)) = self.active {
            // Upgrade probe: one cheaper rung at a time.
            let cheaper = || {
                TransportKind::ALL
                    .into_iter()
                    .filter(move |kind| *kind < active_kind)
            };
            if cheaper().any(|kind| matches!(self.rungs.get(kind).phase, Phase::Connecting(_))) {
                return None;
            }
            let (due, kind) = earliest(self.next_probe_at, &mut cheaper().filter(idle))?;
            return Some(Plan {
                due,
                kind,
                restart: false,
            });
        }

        let origin = self.origin;
        let race_order = || {
            let (cheaper, from_origin): (TransportSet, TransportSet) = (
                TransportKind::ALL
                    .into_iter()
                    .filter(|kind| *kind < origin)
                    .collect(),
                TransportKind::ALL
                    .into_iter()
                    .filter(|kind| *kind >= origin)
                    .collect(),
            );
            from_origin.iter().chain(cheaper.iter())
        };
        let tried = self.tried;
        if let Some((due, kind)) = earliest(
            self.next_start_at,
            &mut race_order()
                .filter(idle)
                .filter(|kind| !tried.contains(*kind)),
        ) {
            return Some(Plan {
                due,
                kind,
                restart: false,
            });
        }
        let in_flight = TransportKind::ALL
            .into_iter()
            .any(|kind| self.rungs.get(kind).phase != Phase::Idle);
        if in_flight {
            return None;
        }
        let (due, kind) = earliest(self.next_start_at, &mut race_order().filter(idle))?;
        Some(Plan {
            due,
            kind,
            restart: true,
        })
    }
}

#[cfg(test)]
mod tests;
