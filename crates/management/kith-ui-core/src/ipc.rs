//! The only daemon client: every read and change the page makes goes through here, to the
//! daemon's socket or, under `--fixture`, to a recorded network.
//!
//! One follower per server keeps [`Live`] current: it subscribes to the daemon's journal,
//! reads a tick every 2 s, and notices when the daemon stops or restarts. Handlers ask the
//! daemon directly with [`Daemon::ask`].

use core::time::Duration;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use kith_ipc::{
    Entry, ErrorCode, IpcClient, IpcError, LinkReport, NetworkSummary, Request, Response, Status,
    TrafficSeries,
};
use kith_types::NodeId;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::MissedTickBehavior;

use crate::Source as Config;
use crate::api::ApiError;
use crate::stream::Live;
use crate::wire::{ApiErrorCode, Boot, Fixture, LinkTick, RateBand, Tick, TrafficTick};
use crate::{Shared, api, fixture, json, now_ms};

/// How often numbers are read.
pub(crate) const TICK: Duration = Duration::from_secs(2);
/// How long to wait before asking a daemon that did not answer again.
const RETRY: Duration = Duration::from_secs(1);
/// How long the daemon may take to send the journal history.
const HISTORY_TIMEOUT: Duration = Duration::from_secs(5);
/// Samples per device in the boot snapshot: five minutes at one every two seconds.
const TAIL: usize = 150;
/// Below this many bytes per second a link is idle.
const LIGHT_FROM: u64 = 1_000;
/// Above this many bytes per second a link is busy.
const BUSY_ABOVE: u64 = 1_000_000;

/// The daemon behind its socket.
#[derive(Debug)]
pub(crate) struct Daemon {
    socket: PathBuf,
}

/// Where the page's data comes from.
#[derive(Debug)]
pub(crate) enum Source {
    Daemon(Daemon),
    Fixture(Box<Fixture>),
}

impl Source {
    pub(crate) fn new(config: Config) -> Self {
        match config {
            Config::Daemon(socket) => Self::Daemon(Daemon { socket }),
            Config::Fixture(fixture) => Self::Fixture(fixture),
        }
    }
}

impl Daemon {
    /// A new connection, after the protocol handshake.
    pub(crate) async fn connect(&self) -> Result<IpcClient, ApiError> {
        IpcClient::connect(&self.socket).await.map_err(|error| {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "every other failure means the daemon is not answering"
            )]
            let code = match &error {
                IpcError::VersionMismatch { .. }
                | IpcError::Remote {
                    code: ErrorCode::VersionMismatch,
                    ..
                } => ApiErrorCode::VersionSkew,
                IpcError::Io(io) if io.kind() == io::ErrorKind::PermissionDenied => {
                    ApiErrorCode::NotInGroup
                }
                _ => ApiErrorCode::DaemonDown,
            };
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, code, error.to_string())
        })
    }

    /// One request on a fresh connection.
    pub(crate) async fn ask(&self, request: Request) -> Result<Response, ApiError> {
        Ok(self.connect().await?.request(request).await?)
    }
}

/// The follower task; stopped when the server is dropped.
#[derive(Debug)]
pub(crate) struct Follower(AbortHandle);

impl Drop for Follower {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts keeping `live` current from `source`. Needs a running tokio runtime.
pub(crate) fn follow(source: &Source, live: Arc<Live>) -> io::Result<Follower> {
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| io::Error::other("the kith-ui router must be built inside a tokio runtime"))?;
    let task = match source {
        Source::Daemon(daemon) => runtime.spawn(follow_daemon(daemon.socket.clone(), live)),
        Source::Fixture(recorded) => runtime.spawn(fixture::replay((**recorded).clone(), live)),
    };
    Ok(Follower(task.abort_handle()))
}

type Entries = mpsc::Receiver<Result<Entry, IpcError>>;

/// Why a subscription did not start.
enum Retry {
    /// The daemon restarted; subscribe again from its oldest entry.
    Now,
    /// The daemon did not answer.
    Later,
}

async fn follow_daemon(socket: PathBuf, live: Arc<Live>) {
    loop {
        match subscribe(&socket, &live).await {
            Ok(entries) => {
                run(&socket, &live, entries).await;
                live.down();
            }
            Err(Retry::Now) => continue,
            Err(Retry::Later) => live.down(),
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Subscribes to the journal and brings `live` up to date with its history.
///
/// The subscription starts at the newest entry `live` keeps. If the daemon still has that
/// entry, it is the same run and only newer entries are added. Otherwise the daemon
/// restarted, and the run is replaced from the daemon's oldest entry.
async fn subscribe(socket: &Path, live: &Live) -> Result<Entries, Retry> {
    let last = live.last_entry();
    let since = last.map(|(seq, _)| seq);
    let client = IpcClient::connect(socket).await.map_err(|_| Retry::Later)?;
    let (next, mut entries) = client.journal(since).await.map_err(|_| Retry::Later)?;
    let history = tokio::time::timeout(HISTORY_TIMEOUT, history(&mut entries, since, next))
        .await
        .ok()
        .flatten()
        .ok_or(Retry::Later)?;
    match last {
        None => live.restart(history, next),
        Some((seq, at_ms))
            if history
                .first()
                .is_some_and(|entry| entry.seq == seq && entry.at_ms == at_ms) =>
        {
            live.extend(history);
        }
        Some(_) => {
            live.forget();
            return Err(Retry::Now);
        }
    }
    Ok(entries)
}

/// The entries the daemon sends before `next`: its history from `since`, or from its
/// oldest entry. `None` when the subscription ends first.
async fn history(entries: &mut Entries, since: Option<u64>, next: u64) -> Option<Vec<Entry>> {
    let mut history = Vec::new();
    let Some(last) = next.checked_sub(1) else {
        return Some(history);
    };
    if since.is_some_and(|from| from > last) {
        return Some(history);
    }
    while let Ok(entry) = entries.recv().await? {
        let done = entry.seq >= last;
        history.push(entry);
        if done {
            return Some(history);
        }
    }
    None
}

/// Applies entries and reads ticks until the journal subscription ends.
///
/// A tick the daemon cannot answer within [`TICK`] is skipped, so a slow daemon delays no
/// entry; the page notices the missing ticks and shows its values as stale.
async fn run(socket: &Path, live: &Live, mut entries: Entries) {
    let mut client: Option<IpcClient> = None;
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            entry = entries.recv() => match entry {
                Some(Ok(entry)) => live.extend([entry]),
                Some(Err(_)) | None => return,
            },
            _ = ticks.tick() => {
                let read = tokio::time::timeout(TICK, async {
                    if client.is_none() {
                        client = IpcClient::connect(socket).await.ok();
                    }
                    match client.as_mut() {
                        Some(connected) => read_tick(connected).await.ok(),
                        None => None,
                    }
                })
                .await
                .ok()
                .flatten();
                match read {
                    Some((tick, series)) => {
                        live.record(|history| history.record(&series));
                        live.tick(tick);
                    }
                    None => client = None,
                }
            }
        }
    }
}

/// How busy a link is, for members, who never see another device's exact rate.
fn band(bytes_per_sec: u64) -> RateBand {
    if bytes_per_sec < LIGHT_FROM {
        RateBand::Idle
    } else if bytes_per_sec <= BUSY_ABOVE {
        RateBand::Light
    } else {
        RateBand::Busy
    }
}

/// One network's link reports as tick links. In a network where this device is a member,
/// links between other devices carry a [`RateBand`] instead of their rate.
fn links(network: &NetworkSummary, own: Option<NodeId>, reports: &[LinkReport]) -> Vec<LinkTick> {
    reports
        .iter()
        .flat_map(|report| {
            let exact = network.admin || own == Some(report.node);
            report.links.iter().map(move |link| LinkTick {
                network: network.id.to_string(),
                from: report.node.to_string(),
                to: link.peer.to_string(),
                path: link.path,
                relay: link.relay.map(|relay| relay.to_string()),
                rtt_ms: link.rtt_ms,
                loss: None,
                rate: exact.then_some(link.bytes_per_sec),
                rate_band: (!exact).then(|| band(link.bytes_per_sec)),
                age_secs: report.age_secs,
            })
        })
        .collect()
}

/// Counters and links now, and the traffic history the daemon keeps.
async fn read_tick(client: &mut IpcClient) -> Result<(Tick, Vec<TrafficSeries>), IpcError> {
    let Response::Traffic(series) = client.request(Request::Traffic).await? else {
        return Err(IpcError::Unexpected);
    };
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(IpcError::Unexpected);
    };
    let traffic = series
        .iter()
        .filter_map(|one| {
            let last = one.samples.last()?;
            Some(TrafficTick {
                network: one.network.to_string(),
                node: one.node.to_string(),
                rx: last.rx_bytes,
                tx: last.tx_bytes,
            })
        })
        .collect();
    Ok((
        Tick {
            at_ms: now_ms(),
            traffic,
            links: read_links(client, &status).await?,
        },
        series,
    ))
}

async fn read_links(client: &mut IpcClient, status: &Status) -> Result<Vec<LinkTick>, IpcError> {
    let mut all = Vec::new();
    for network in &status.networks {
        let Response::Topology(reports) = client
            .request(Request::Topology {
                network: network.id,
            })
            .await?
        else {
            return Err(IpcError::Unexpected);
        };
        all.extend(links(network, status.node, &reports));
    }
    Ok(all)
}

/// The boot snapshot: `boot` and `seq` are read first, so every entry after `seq` reaches
/// the page on the stream; traffic and the last tick come from what the server keeps.
pub(crate) async fn snapshot(shared: &Shared) -> Result<Boot, ApiError> {
    let (boot, seq) = shared.live.position().await;
    let mut snapshot = api::status::snapshot(shared).await?;
    snapshot.boot = boot;
    snapshot.seq = seq;
    let traffic = shared.live.traffic(TAIL);
    if !traffic.is_empty() {
        snapshot.traffic_tail = json::to_value(&traffic)?;
    }
    if let Some(tick) = shared.live.last_tick() {
        snapshot.last_tick = Some(tick);
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use kith_ipc::{LinkInfo, PathKind};
    use kith_types::NetworkId;

    use super::*;

    #[test]
    fn members_see_bands_for_links_between_other_devices() {
        let own = NodeId::from_bytes([1; 32]);
        let other = NodeId::from_bytes([2; 32]);
        let report = |node, bytes_per_sec| LinkReport {
            node,
            name: String::new(),
            age_secs: 0,
            links: vec![LinkInfo {
                peer: NodeId::from_bytes([3; 32]),
                peer_name: String::new(),
                path: PathKind::Direct,
                relay: None,
                rtt_ms: Some(20),
                bytes_per_sec,
            }],
        };
        let mut network = NetworkSummary {
            id: NetworkId::from_bytes([9; 32]),
            name: "home".into(),
            peers_online: 0,
            peers_total: 0,
            relays: 0,
            problems: Vec::new(),
            admin: false,
        };
        let reports = [report(own, 5_000), report(other, 2_000_000)];
        let shown: Vec<_> = links(&network, Some(own), &reports)
            .into_iter()
            .map(|link| (link.rate, link.rate_band))
            .collect();
        assert_eq!(shown, [(Some(5_000), None), (None, Some(RateBand::Busy))]);
        network.admin = true;
        assert!(
            links(&network, Some(own), &reports)
                .iter()
                .all(|link| link.rate.is_some() && link.rate_band.is_none())
        );
        assert_eq!(
            [band(999), band(1_000), band(1_000_000), band(1_000_001)],
            [
                RateBand::Idle,
                RateBand::Light,
                RateBand::Light,
                RateBand::Busy
            ]
        );
    }
}
