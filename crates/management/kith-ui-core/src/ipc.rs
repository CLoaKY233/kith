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
use kith_ipc::{Entry, ErrorCode, IpcClient, IpcError, Request, Response, Status, TrafficSeries};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::MissedTickBehavior;

use crate::Source as Config;
use crate::api::ApiError;
use crate::stream::Live;
use crate::wire::{ApiErrorCode, Boot, Fixture, LinkTick, Tick, TrafficTick};
use crate::{Shared, api, fixture, json, now_ms};

/// How often numbers are read.
pub(crate) const TICK: Duration = Duration::from_secs(2);
/// How long to wait before asking a daemon that did not answer again.
const RETRY: Duration = Duration::from_secs(1);
/// How long the daemon may take to send the journal history.
const HISTORY_TIMEOUT: Duration = Duration::from_secs(5);
/// Samples per device in the boot snapshot: five minutes at one every two seconds.
const TAIL: usize = 150;

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
        all.extend(api::status::links(network, &reports, status.node));
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
