//! `kith-ui --fixture`: a recorded network served as if a daemon answered, so the page is
//! built and tested without one.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::now_ms;
use crate::wire::{Boot, Fixture, LinkTick, Tick, TrafficTick};

/// Samples per device in the boot snapshot.
const TAIL: usize = 150;

/// Why a fixture could not be loaded.
#[derive(Debug)]
pub enum FixtureError {
    /// The file could not be read.
    Read(std::io::Error),
    /// The file is not a fixture.
    Parse(serde_json::Error),
}

impl core::error::Error for FixtureError {}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read the fixture: {error}"),
            Self::Parse(error) => write!(f, "not a fixture: {error}"),
        }
    }
}

/// Adds `offset` to every time in milliseconds, so a recording plays as if made now.
fn shift(value: &mut Value, offset: i64) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                let is_time = key.ends_with("_ms") || key == "paused_until";
                if is_time
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
    let text = std::fs::read_to_string(path).map_err(FixtureError::Read)?;
    let mut value: Value = serde_json::from_str(&text).map_err(FixtureError::Parse)?;
    let recorded = value
        .get("recorded_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let offset = i64::try_from(now_ms())
        .unwrap_or(i64::MAX)
        .saturating_sub(i64::try_from(recorded).unwrap_or(i64::MAX));
    shift(&mut value, offset);
    serde_json::from_value(value).map_err(FixtureError::Parse)
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

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use kith_ipc::{Entry, PeerInfo, Status, TrafficSeries};

    use super::*;
    use crate::json;

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
}
