//! Simulated-time tests; the harness plays the driver.

#![expect(clippy::arithmetic_side_effects, reason = "test counters stay tiny")]

use alloc::{collections::BTreeSet, vec::Vec};
use core::time::Duration;

use proptest::prelude::*;

use super::*;
use TransportKind::{Quic, Udp, UdpObfuscated, WebSocket};

const RTT: Duration = Duration::from_millis(20);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn network(byte: u8) -> NetworkFingerprint {
    NetworkFingerprint([byte; 32])
}

/// An action with the attempt id dropped, for readable expectations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Start(TransportKind),
    Close(LinkId),
}

struct Sim {
    ladder: Ladder,
    now: Duration,
    /// Every action with the time it was emitted.
    log: Vec<(Duration, Action)>,
}

impl Sim {
    fn new(config: LadderConfig) -> Self {
        Self {
            ladder: Ladder::new(config).unwrap(),
            now: Duration::ZERO,
            log: Vec::new(),
        }
    }

    /// Runs the ladder until `until`, honouring every deadline on the way,
    /// and returns the actions emitted. Checks that `poll_timeout` never
    /// promises work `poll_action` does not deliver, and vice versa.
    fn run_until(&mut self, until: Duration) -> Vec<Act> {
        let start = self.log.len();
        loop {
            self.drain();
            match self.ladder.poll_timeout() {
                Some(deadline) if deadline <= until => self.now = self.now.max(deadline),
                _ => break,
            }
        }
        self.now = until;
        self.drain();
        self.log[start..]
            .iter()
            .map(|(_, action)| match *action {
                Action::Start { kind, .. } => Act::Start(kind),
                Action::Close(link) => Act::Close(link),
            })
            .collect()
    }

    fn drain(&mut self) {
        for _ in 0..256 {
            let Some(action) = self.ladder.poll_action(self.now) else {
                let next = self.ladder.poll_timeout();
                assert!(
                    next.is_none_or(|deadline| deadline > self.now),
                    "timeout {next:?} due but no action"
                );
                return;
            };
            self.log.push((self.now, action));
        }
        panic!("action storm at {:?}", self.now);
    }

    fn event(&mut self, event: LinkEvent) {
        self.ladder.handle_event(self.now, event);
    }

    /// The most recent attempt started on `kind`.
    fn attempt(&self, kind: TransportKind) -> AttemptId {
        self.attempts()
            .filter(|(_, started)| *started == kind)
            .last()
            .map(|(attempt, _)| attempt)
            .unwrap()
    }

    fn attempts(&self) -> impl Iterator<Item = (AttemptId, TransportKind)> + '_ {
        self.log.iter().filter_map(|(_, action)| match *action {
            Action::Start { kind, attempt } => Some((attempt, kind)),
            Action::Close(_) => None,
        })
    }

    fn up(&mut self, link: u64, kind: TransportKind) {
        let attempt = self.attempt(kind);
        self.event(LinkEvent::Up {
            link: LinkId(link),
            attempt,
            rtt: RTT,
        });
    }

    fn fail(&mut self, kind: TransportKind) {
        let attempt = self.attempt(kind);
        self.event(LinkEvent::Failed {
            attempt,
            reason: FailureReason::Timeout,
        });
    }

    fn starts(&self) -> Vec<(Duration, TransportKind)> {
        self.log
            .iter()
            .filter_map(|(at, action)| match *action {
                Action::Start { kind, .. } => Some((*at, kind)),
                Action::Close(_) => None,
            })
            .collect()
    }

    fn states(&self) -> Vec<RungState> {
        TransportKind::ALL
            .iter()
            .map(|kind| self.ladder.rung_state(*kind))
            .collect()
    }
}

#[test]
fn working_udp_is_kept_and_no_costlier_rung_is_ever_started() {
    let mut sim = Sim::new(LadderConfig::default());
    assert_eq!(sim.run_until(ms(0)), [Act::Start(Udp)]);
    sim.up(1, Udp);
    assert_eq!(sim.run_until(ms(3_600_000)), []);
    assert_eq!(sim.ladder.active_link(), Some((LinkId(1), Udp)));
    assert_eq!(sim.ladder.active_rtt(), Some(RTT));
}

#[test]
fn silently_blocked_udp_climbs_one_rung_per_stagger_and_stops_at_quic() {
    let mut sim = Sim::new(LadderConfig::default());
    sim.run_until(ms(600));
    sim.up(1, Quic);
    sim.run_until(ms(10_000));
    assert_eq!(
        sim.starts(),
        [(ms(0), Udp), (ms(250), UdpObfuscated), (ms(500), Quic)]
    );
    assert_eq!(sim.ladder.active_link(), Some((LinkId(1), Quic)));
}

#[test]
fn reported_failures_skip_the_stagger_and_climb_to_websocket() {
    let mut sim = Sim::new(LadderConfig::default());
    for (at, kind) in [(10, Udp), (20, UdpObfuscated), (30, Quic)] {
        sim.run_until(ms(at));
        sim.fail(kind);
    }
    sim.run_until(ms(30));
    sim.up(1, WebSocket);
    sim.run_until(ms(900));
    // Reality is not in the default set, so the race ends at WebSocket.
    assert_eq!(
        sim.starts(),
        [
            (ms(0), Udp),
            (ms(10), UdpObfuscated),
            (ms(20), Quic),
            (ms(30), WebSocket)
        ]
    );
    assert_eq!(sim.ladder.active_link(), Some((LinkId(1), WebSocket)));
}

#[test]
fn recovered_cheaper_rung_is_adopted_and_costlier_link_closed_after_grace() {
    let config = LadderConfig {
        enabled: TransportSet::EMPTY.with(Udp).with(Quic),
        ..LadderConfig::default()
    };
    let mut sim = Sim::new(config);
    sim.run_until(ms(0));
    sim.fail(Udp);
    sim.run_until(ms(1));
    sim.up(1, Quic);

    let probe_at = ms(1).saturating_add(UPGRADE_PROBE_INTERVAL);
    assert_eq!(sim.run_until(probe_at), [Act::Start(Udp)]);
    sim.up(2, Udp);
    assert_eq!(sim.ladder.active_link(), Some((LinkId(2), Udp)));

    let grace_end = probe_at.saturating_add(SUPERSEDE_GRACE);
    assert_eq!(sim.run_until(grace_end.saturating_sub(ms(1))), []);
    assert_eq!(sim.run_until(grace_end), [Act::Close(LinkId(1))]);
    assert_eq!(sim.ladder.rung_state(Quic), RungState::Idle);
}

#[test]
fn losing_the_active_link_falls_back_to_the_superseded_one_without_reconnecting() {
    let mut sim = Sim::new(LadderConfig::default());
    sim.run_until(ms(500));
    sim.up(1, Quic);
    sim.up(2, Udp); // Udp's attempt was still in flight.
    sim.event(LinkEvent::Down {
        link: LinkId(2),
        reason: FailureReason::Closed,
    });
    assert_eq!(sim.ladder.active_link(), Some((LinkId(1), Quic)));
    // The fallback is no longer superseded: it must not be closed after the grace period.
    let closes = sim.run_until(ms(30_000));
    assert!(!closes.contains(&Act::Close(LinkId(1))), "{closes:?}");
}

#[test]
fn backoff_doubles_per_failure_up_to_the_cap() {
    let config = LadderConfig {
        enabled: TransportSet::EMPTY.with(Udp),
        ..LadderConfig::default()
    };
    let mut sim = Sim::new(config);
    for _ in 0..9 {
        sim.run_until(sim.ladder.poll_timeout().unwrap());
        sim.fail(Udp);
    }
    let times: Vec<u64> = sim.starts().iter().map(|(at, _)| at.as_secs()).collect();
    let gaps: Vec<u64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert_eq!(gaps, [1, 2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(sim.ladder.restarts(), 8);
}

#[test]
fn known_network_starts_at_cached_rung_and_probes_for_an_upgrade() {
    let config = LadderConfig {
        enabled: TransportSet::EMPTY.with(Udp).with(WebSocket),
        ..LadderConfig::default()
    };
    let mut sim = Sim::new(config);
    sim.ladder.set_fingerprint(sim.now, network(1));
    sim.run_until(ms(250));
    sim.up(1, WebSocket);
    sim.fail(Udp);

    sim.ladder.set_fingerprint(sim.now, network(2));
    sim.run_until(ms(300));
    sim.up(2, Udp);
    sim.ladder.set_fingerprint(sim.now, network(1));
    // Back on the first network: straight to WebSocket, no stagger.
    assert_eq!(
        sim.run_until(ms(300)),
        [Act::Close(LinkId(2)), Act::Start(WebSocket)]
    );
    sim.up(3, WebSocket);
    let probe_at = ms(300).saturating_add(UPGRADE_PROBE_INTERVAL);
    assert_eq!(sim.run_until(probe_at), [Act::Start(Udp)]);
}

#[test]
fn roaming_closes_every_link_and_restarts_from_the_cheapest_rung() {
    let mut sim = Sim::new(LadderConfig::default());
    sim.ladder.set_fingerprint(sim.now, network(1));
    sim.run_until(ms(500));
    sim.up(1, Quic);
    sim.ladder.set_fingerprint(sim.now, network(2));
    assert_eq!(sim.ladder.active_link(), None);
    assert_eq!(
        sim.run_until(ms(500)),
        [Act::Close(LinkId(1)), Act::Start(Udp)]
    );
    // An attempt abandoned on the old network reports late; it must not win.
    sim.up(9, UdpObfuscated);
    assert_eq!(sim.run_until(ms(500)), [Act::Close(LinkId(9))]);
}

#[test]
fn rejects_configs_that_could_never_connect_or_would_spin() {
    let base = LadderConfig::default();
    let reject = |config: LadderConfig| Ladder::new(config).err();
    assert_eq!(
        reject(LadderConfig {
            enabled: TransportSet::EMPTY,
            ..base
        }),
        Some(ConfigError::NoTransports)
    );
    assert_eq!(
        reject(LadderConfig {
            backoff_base: Duration::ZERO,
            ..base
        }),
        Some(ConfigError::ZeroBackoff)
    );
    assert_eq!(
        reject(LadderConfig {
            upgrade_probe_interval: Duration::ZERO,
            ..base
        }),
        Some(ConfigError::ZeroProbeInterval)
    );
}

/// One driver-side step in the property test.
#[derive(Debug, Clone)]
enum Step {
    Advance(u64),
    /// Up with a fresh link for the n-th attempt ever started (often stale).
    Up(usize),
    /// Failure report for the n-th attempt ever started (often stale).
    Failed(usize),
    /// This many Ups for attempts that were never started, without draining in between.
    StrayBurst(usize),
    /// Down for the n-th open link, modulo the number of open links.
    Down(usize),
    Roam(u8),
    Enable(u8),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0_u64..120_000).prop_map(Step::Advance),
        3 => any::<usize>().prop_map(Step::Up),
        3 => any::<usize>().prop_map(Step::Failed),
        1 => (1_usize..40).prop_map(Step::StrayBurst),
        2 => any::<usize>().prop_map(Step::Down),
        1 => (0_u8..3).prop_map(Step::Roam),
        1 => (1_u8..32).prop_map(Step::Enable),
    ]
}

fn set_from_mask(mask: u8) -> TransportSet {
    TransportKind::ALL
        .into_iter()
        .filter(|kind| mask & (1 << kind.index()) != 0)
        .collect()
}

/// Picks an attempt, biased towards the most recent ones so that live
/// answers are common, while older (stale) ones still appear.
fn pick_attempt(sim: &Sim, pick: usize) -> Option<(AttemptId, TransportKind)> {
    let attempts: Vec<_> = sim.attempts().collect();
    let window = if pick.is_multiple_of(2) {
        attempts.len().min(3)
    } else {
        attempts.len()
    };
    let offset = (pick / 2) % window.max(1);
    attempts
        .len()
        .checked_sub(1 + offset)
        .and_then(|index| attempts.get(index).copied())
}

/// Whether the ladder is still waiting for `attempt`.
fn is_live(sim: &Sim, attempt: AttemptId, kind: TransportKind) -> bool {
    sim.attempt(kind) == attempt && sim.ladder.rung_state(kind) == RungState::Connecting
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The ladder against an adversarial driver: answers arrive late, after
    /// roaming or restarts, for attempts never made, and in bursts, while the
    /// network and policy change at random.
    #[test]
    fn random_event_sequences_keep_every_invariant(
        mask in 1_u8..32,
        steps in proptest::collection::vec(step(), 0..120),
    ) {
        let mut sim = Sim::new(LadderConfig { enabled: set_from_mask(mask), ..LadderConfig::default() });
        let mut open: BTreeSet<LinkId> = BTreeSet::new();
        let mut next_link = 0_u64;
        let mut inputs = 1_usize;
        let mut checked = 0;

        for step in steps {
            match step {
                Step::Advance(millis) => {
                    let until = sim.now.saturating_add(ms(millis));
                    // Deadline by deadline, so each start is checked against the state before it.
                    let mut cursor = checked;
                    loop {
                        let deadline = sim.ladder.poll_timeout().filter(|deadline| *deadline <= until);
                        let states = sim.states();
                        let enabled = sim.ladder.enabled();
                        sim.run_until(deadline.unwrap_or(until).max(sim.now));
                        for (_, action) in &sim.log[cursor..] {
                            if let Action::Start { kind, .. } = *action {
                                prop_assert!(enabled.contains(kind), "started disabled {kind}");
                                prop_assert_eq!(states[kind.index()], RungState::Idle, "{} restarted while busy", kind);
                            }
                        }
                        cursor = sim.log.len();
                        if deadline.is_none() { break; }
                    }
                    prop_assert!(sim.ladder.poll_action(until).is_none());
                }
                Step::Up(pick) => {
                    let Some((attempt, kind)) = pick_attempt(&sim, pick) else { continue };
                    let link = LinkId(next_link);
                    next_link += 1;
                    open.insert(link);
                    let live = is_live(&sim, attempt, kind);
                    let (states, active) = (sim.states(), sim.ladder.active_link());
                    sim.event(LinkEvent::Up { link, attempt, rtt: RTT });
                    if !live {
                        // A stale answer changes nothing but gets its link closed.
                        prop_assert_eq!(sim.states(), states);
                        prop_assert_eq!(sim.ladder.active_link(), active);
                        prop_assert_eq!(sim.ladder.poll_action(sim.now), Some(Action::Close(link)));
                        sim.log.push((sim.now, Action::Close(link)));
                    }
                    inputs += 1;
                }
                Step::Failed(pick) => {
                    let Some((attempt, kind)) = pick_attempt(&sim, pick) else { continue };
                    let live = is_live(&sim, attempt, kind);
                    let before = alloc::format!("{:?}", sim.ladder);
                    sim.event(LinkEvent::Failed { attempt, reason: FailureReason::Timeout });
                    if !live {
                        // No effect at all, in particular on backoff.
                        prop_assert_eq!(alloc::format!("{:?}", sim.ladder), before);
                    }
                    inputs += 1;
                }
                Step::StrayBurst(count) => {
                    for _ in 0..count {
                        let link = LinkId(next_link);
                        next_link += 1;
                        open.insert(link);
                        sim.event(LinkEvent::Up { link, attempt: AttemptId(u64::MAX - next_link), rtt: RTT });
                    }
                    inputs += 1;
                }
                Step::Down(pick) => {
                    if let Some(link) = open.iter().nth(pick % open.len().max(1)).copied() {
                        open.remove(&link);
                        sim.event(LinkEvent::Down { link, reason: FailureReason::Closed });
                        inputs += 1;
                    }
                }
                Step::Roam(byte) => {
                    sim.ladder.set_fingerprint(sim.now, network(byte));
                    inputs += 1;
                }
                Step::Enable(mask) => {
                    sim.ladder.set_enabled(sim.now, set_from_mask(mask)).unwrap();
                    inputs += 1;
                }
            }
            sim.run_until(sim.now);
            for (_, action) in &sim.log[checked..] {
                match *action {
                    Action::Start { kind, .. } => prop_assert!(sim.ladder.enabled().contains(kind)),
                    Action::Close(link) => prop_assert!(open.contains(&link), "closed {link:?} twice or never opened"),
                }
                if let Action::Close(link) = *action {
                    open.remove(&link);
                }
            }
            checked = sim.log.len();

            // No leaks: every link the driver holds open is one the ladder holds.
            let held: BTreeSet<LinkId> = sim.states().into_iter().filter_map(|state| match state {
                RungState::Up(link) => Some(link),
                RungState::Idle | RungState::Connecting => None,
            }).collect();
            prop_assert_eq!(&held, &open);

            // The active link is the cheapest held link, and exists whenever one is held.
            let cheapest_up = TransportKind::ALL.iter().find_map(|kind| match sim.ladder.rung_state(*kind) {
                RungState::Up(link) => Some((link, *kind)),
                RungState::Idle | RungState::Connecting => None,
            });
            prop_assert_eq!(sim.ladder.active_link(), cheapest_up);
            if let Some((_, kind)) = cheapest_up {
                prop_assert!(sim.ladder.enabled().contains(kind));
            }
        }

        let ids: Vec<AttemptId> = sim.attempts().map(|(attempt, _)| attempt).collect();
        prop_assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

        // Every start needs an idle rung, and only inputs make rungs idle,
        // so starts are bounded by inputs regardless of elapsed time.
        let starts = sim.starts().len();
        prop_assert!(starts <= TransportKind::COUNT * inputs, "{starts} starts for {inputs} inputs");
    }
}
