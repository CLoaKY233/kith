//! `kith-ui --fixture`: a recorded network served as if a daemon answered, so the page is
//! built and tested without one.
//!
//! The stream replays the recording through [`Live`], the same path a daemon's journal
//! takes: the recorded journal is the history a page can resume from, ticks advance the
//! recorded traffic, and the recorded activity is played again as new entries, 60 times
//! faster than it happened, so the page always has something live to show. Changes go to
//! [`crate::sim`], which edits the recording and writes their entries.

use core::time::Duration;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

use kith_ipc::{Entry, EntryKind, TrafficSample, TrafficSeries};
use kith_types::{NetworkId, NodeId};
use serde_json::Value;
use tokio::time::{Instant, MissedTickBehavior};

use crate::ipc::TICK;
use crate::stream::Live;
use crate::wire::{Boot, Fixture, LinkTick, Tick, TrafficTick};
use crate::{json, now_ms};

/// Samples per device in the boot snapshot.
const TAIL: usize = 150;
/// How much faster than recorded the journal replays: a day in 24 minutes.
const SPEEDUP: u64 = 60;
/// The shortest wait between replayed entries, so a recorded burst stays readable.
const REPLAY_MIN: Duration = Duration::from_millis(500);
/// The longest wait between replayed entries, so a recorded quiet hour stays short.
const REPLAY_MAX: Duration = Duration::from_secs(20);
/// The largest fixture read; the recorded office is well under a megabyte.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Why a fixture could not be loaded.
#[derive(Debug)]
pub enum FixtureError {
    /// The file could not be read.
    Read(std::io::Error),
    /// The file is not a fixture.
    Parse(serde_json::Error),
    /// The file is larger than the fixture size limit.
    TooLarge,
    /// The boot id is not 1 to 64 ASCII letters and digits, so it cannot name events.
    Boot,
}

impl core::error::Error for FixtureError {}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read the fixture: {error}"),
            Self::Parse(error) => write!(f, "not a fixture: {error}"),
            Self::TooLarge => write!(f, "not a fixture: larger than {MAX_BYTES} bytes"),
            Self::Boot => f.write_str("not a fixture: `boot` must be 1 to 64 letters and digits"),
        }
    }
}

/// Whether `key` holds a moment in Unix milliseconds rather than a duration such as
/// `rtt_ms`.
fn is_moment(key: &str) -> bool {
    key == "at_ms"
        || key.ends_with("_at_ms")
        || matches!(
            key,
            "first_ms" | "last_ms" | "server_time_ms" | "paused_until"
        )
}

/// Adds `offset` to every moment, so a recording plays as if made now.
fn shift(value: &mut Value, offset: i64) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if is_moment(key)
                    && let Some(shifted) =
                        field.as_u64().and_then(|ms| ms.checked_add_signed(offset))
                {
                    *field = Value::from(shifted);
                } else {
                    shift(field, offset);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| shift(item, offset)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Reads a fixture and moves its times so that it was recorded just now.
pub fn load_fixture(path: &Path) -> Result<Fixture, FixtureError> {
    let file = std::fs::File::open(path).map_err(FixtureError::Read)?;
    let mut text = String::new();
    file.take(MAX_BYTES.saturating_add(1))
        .read_to_string(&mut text)
        .map_err(FixtureError::Read)?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(FixtureError::TooLarge);
    }
    let mut value: Value = serde_json::from_str(&text).map_err(FixtureError::Parse)?;
    let recorded = value
        .get("recorded_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let offset = i64::try_from(now_ms())
        .unwrap_or(i64::MAX)
        .saturating_sub(i64::try_from(recorded).unwrap_or(i64::MAX));
    shift(&mut value, offset);
    let fixture: Fixture = serde_json::from_value(value).map_err(FixtureError::Parse)?;
    let boot_ok = (1..=64).contains(&fixture.boot.len())
        && fixture
            .boot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric());
    if !boot_ok {
        return Err(FixtureError::Boot);
    }
    Ok(fixture)
}

/// The last `TAIL` samples of each series.
fn tail(traffic: &Value) -> Value {
    let Some(series) = traffic.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        series
            .iter()
            .map(|one| {
                let mut one = one.clone();
                if let Some(samples) = one.get_mut("samples").and_then(Value::as_array_mut) {
                    let keep = samples.len().saturating_sub(TAIL);
                    samples.drain(..keep);
                }
                one
            })
            .collect(),
    )
}

/// The last counters of each series, as a tick reports them.
fn counters(traffic: &Value) -> Vec<TrafficTick> {
    traffic
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|one| {
            let last = one.get("samples")?.as_array()?.last()?;
            Some(TrafficTick {
                network: one.get("network")?.as_str()?.to_owned(),
                node: one.get("node")?.as_str()?.to_owned(),
                rx: last.get("rx_bytes")?.as_u64()?,
                tx: last.get("tx_bytes")?.as_u64()?,
            })
        })
        .collect()
}

fn links(fixture: &Fixture) -> Vec<LinkTick> {
    fixture.links.values().flatten().cloned().collect()
}

/// The fixture's latest journal sequence number.
fn last_seq(fixture: &Fixture) -> u64 {
    fixture
        .journal
        .as_array()
        .and_then(|entries| entries.last())
        .and_then(|entry| entry.get("seq"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

/// The snapshot the page boots from.
pub(crate) fn boot(fixture: &Fixture) -> Boot {
    Boot {
        boot: fixture.boot.clone(),
        seq: last_seq(fixture),
        server_time_ms: now_ms(),
        status: fixture.status.clone(),
        peers: fixture.peers.clone(),
        traffic_tail: tail(&fixture.traffic),
        pending: fixture.pending.clone(),
        versions: fixture.status.versions.clone(),
        last_tick: Some(Tick {
            at_ms: now_ms(),
            traffic: counters(&fixture.traffic),
            links: links(fixture),
        }),
    }
}

/// Plays the recorded traffic in a loop: each tick adds the next recorded step to the
/// counters, so rates look like the recording.
#[derive(Debug)]
pub(crate) struct Player {
    steps: BTreeMap<(String, String), Vec<(u64, u64)>>,
    totals: Vec<TrafficTick>,
    links: Vec<LinkTick>,
    turn: usize,
}

impl Player {
    pub(crate) fn new(fixture: &Fixture) -> Self {
        let mut steps = BTreeMap::new();
        for one in fixture.traffic.as_array().into_iter().flatten() {
            let (Some(network), Some(node), Some(samples)) = (
                one.get("network").and_then(Value::as_str),
                one.get("node").and_then(Value::as_str),
                one.get("samples").and_then(Value::as_array),
            ) else {
                continue;
            };
            let counts: Vec<(u64, u64)> = samples
                .iter()
                .filter_map(|sample| {
                    Some((
                        sample.get("rx_bytes")?.as_u64()?,
                        sample.get("tx_bytes")?.as_u64()?,
                    ))
                })
                .collect();
            let deltas = counts
                .windows(2)
                .filter_map(|pair| match pair {
                    [(rx0, tx0), (rx1, tx1)] => {
                        Some((rx1.saturating_sub(*rx0), tx1.saturating_sub(*tx0)))
                    }
                    _ => None,
                })
                .collect();
            steps.insert((network.to_owned(), node.to_owned()), deltas);
        }
        Self {
            steps,
            totals: counters(&fixture.traffic),
            links: links(fixture),
            turn: 0,
        }
    }

    /// The next tick.
    pub(crate) fn tick(&mut self) -> Tick {
        for total in &mut self.totals {
            let step = self
                .steps
                .get(&(total.network.clone(), total.node.clone()))
                .and_then(|deltas| deltas.get(self.turn % deltas.len().max(1)))
                .copied()
                .unwrap_or_default();
            total.rx = total.rx.saturating_add(step.0);
            total.tx = total.tx.saturating_add(step.1);
        }
        self.turn = self.turn.wrapping_add(1);
        Tick {
            at_ms: now_ms(),
            traffic: self.totals.clone(),
            links: self.links.clone(),
        }
    }
}

/// The wait before replaying the recorded entry at `turn`: its recorded distance from the
/// entry before it, sped up and kept between [`REPLAY_MIN`] and [`REPLAY_MAX`].
fn replay_delay(recorded: &[Entry], turn: usize) -> Duration {
    let count = recorded.len().max(1);
    let index = turn % count;
    let before = (index + count - 1) % count;
    let gap = match (recorded.get(index), recorded.get(before)) {
        (Some(entry), Some(previous)) => entry.at_ms.saturating_sub(previous.at_ms) / SPEEDUP,
        _ => u64::MAX,
    };
    Duration::from_millis(gap).clamp(REPLAY_MIN, REPLAY_MAX)
}

/// Whether a recorded entry only reports activity, so replaying it changes no list.
const fn is_activity(kind: &EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::PeerOnline { .. }
            | EntryKind::PeerOffline { .. }
            | EntryKind::PathChanged { .. }
            | EntryKind::EndpointChanged { .. }
            | EntryKind::Dropped { .. }
            | EntryKind::Warning { .. }
            | EntryKind::Log { .. }
    )
}

/// Feeds `live` from the recording for as long as the server runs.
pub(crate) async fn replay(fixture: Fixture, live: Arc<Live>) {
    let recorded: Vec<Entry> = fixture
        .journal
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| json::unshape_entry(entry).ok())
        .collect();
    let next = recorded
        .last()
        .map_or(0, |entry| entry.seq.saturating_add(1));
    // Replayed entries only show activity: membership, names and the exit change only
    // when the page asks the simulator, so the lists stay true to the stream.
    let replayed: Vec<Entry> = recorded
        .iter()
        .filter(|entry| is_activity(&entry.kind))
        .cloned()
        .collect();
    live.restart(recorded, next);
    if let Ok(series) = json::unshape::<Vec<TrafficSeries>>(&fixture.traffic) {
        live.record(|history| history.record(&series));
    }
    let mut player = Player::new(&fixture);
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut turn = 0;
    let wait = tokio::time::sleep(replay_delay(&replayed, turn));
    tokio::pin!(wait);
    loop {
        tokio::select! {
            _ = ticks.tick() => {
                let tick = player.tick();
                live.record(|history| {
                    for counters in &tick.traffic {
                        if let (Ok(network), Ok(node)) = (
                            counters.network.parse::<NetworkId>(),
                            counters.node.parse::<NodeId>(),
                        ) {
                            history.add(network, node, TrafficSample {
                                at_ms: tick.at_ms,
                                rx_bytes: counters.rx,
                                tx_bytes: counters.tx,
                            });
                        }
                    }
                });
                live.tick(tick);
            }
            () = &mut wait => {
                if let Some(entry) = replayed.get(turn % replayed.len().max(1)) {
                    live.push(entry.kind.clone());
                }
                turn = turn.wrapping_add(1);
                wait.as_mut().reset(Instant::now() + replay_delay(&replayed, turn));
            }
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use kith_ipc::{PeerInfo, Status};

    use super::*;

    type TestResult = Result<(), Box<dyn core::error::Error>>;

    fn office() -> Result<Fixture, FixtureError> {
        load_fixture(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../kith-ui/web/fixtures/office.json"),
        )
    }

    /// The fixture is exactly what the server would send for real IPC values, so the page
    /// built against it works against a daemon.
    #[test]
    fn the_office_fixture_is_what_a_daemon_would_produce() -> TestResult {
        let fixture = office()?;
        let status: Status = json::unshape(&fixture.status.status)?;
        assert_eq!(json::to_value(&status)?, fixture.status.status);
        for peers in fixture.peers.values() {
            let parsed: Vec<PeerInfo> = json::unshape(peers)?;
            assert_eq!(&json::to_value(&parsed)?, peers);
        }
        let traffic: Vec<TrafficSeries> = json::unshape(&fixture.traffic)?;
        assert_eq!(json::to_value(&traffic)?, fixture.traffic);
        let entries = fixture.journal.as_array().ok_or("journal is not a list")?;
        let parsed = entries
            .iter()
            .map(json::unshape_entry)
            .collect::<Result<Vec<Entry>, _>>()?;
        assert_eq!(&json::entries(&parsed)?, &fixture.journal);
        assert!(
            parsed.windows(2).all(|pair| match pair {
                [a, b] => a.seq < b.seq && a.at_ms <= b.at_ms,
                _ => true,
            }),
            "the journal is in order"
        );
        Ok(())
    }

    #[test]
    fn shifting_moves_moments_but_not_durations() {
        let mut value = serde_json::json!({
            "at_ms": 1_000,
            "expires_at_ms": 2_000,
            "paused_until": 3_000,
            "links": [{ "rtt_ms": 40, "last_ms": 4_000 }],
        });
        shift(&mut value, 86_400_000 * 60);
        assert_eq!(value["at_ms"], 5_184_001_000_u64);
        assert_eq!(value["expires_at_ms"], 5_184_002_000_u64);
        assert_eq!(value["paused_until"], 5_184_003_000_u64);
        assert_eq!(value["links"][0]["rtt_ms"], 40);
        assert_eq!(value["links"][0]["last_ms"], 5_184_004_000_u64);
    }

    #[test]
    fn the_player_only_counts_up() -> TestResult {
        let fixture = office()?;
        let mut player = Player::new(&fixture);
        let first = player.tick();
        let second = player.tick();
        for (a, b) in first.traffic.iter().zip(&second.traffic) {
            assert!(b.rx >= a.rx && b.tx >= a.tx);
        }
        assert_ne!(first.links.len(), 0);
        Ok(())
    }

    #[test]
    fn replay_keeps_the_recorded_rhythm_within_bounds() {
        let at = |at_ms| Entry {
            seq: 0,
            at_ms,
            kind: EntryKind::Log {
                level: "info".into(),
                target: "test".into(),
                message: String::new(),
            },
        };
        let recorded = [at(0), at(300_000), at(300_010), at(99_000_000)];
        let waits: Vec<Duration> = (0..5).map(|turn| replay_delay(&recorded, turn)).collect();
        assert_eq!(
            waits,
            [
                REPLAY_MIN,
                Duration::from_secs(5),
                REPLAY_MIN,
                REPLAY_MAX,
                REPLAY_MIN
            ]
        );
        assert_eq!(replay_delay(&[], 0), REPLAY_MAX);
    }
}
