//! The only daemon client: every read and change the page makes goes through here, to the
//! daemon's socket or, under `--fixture`, to the simulator.
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
use crate::sim::Sim;
use crate::stream::Live;
use crate::wire::{ApiErrorCode, Boot, LinkTick, Tick, TrafficTick};
use crate::{Shared, api, fixture, json, now_ms};

/// How often numbers are read.
pub(crate) const TICK: Duration = Duration::from_secs(2);
/// How long to wait before asking a daemon that did not answer again.
const RETRY: Duration = Duration::from_secs(1);
/// How long to wait before asking a daemon that speaks another protocol again: it only
/// changes when one side is upgraded.
const SKEW_RETRY: Duration = Duration::from_secs(5);
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
    Fixture(Box<Sim>),
}

impl Source {
    pub(crate) fn new(config: Config) -> Self {
        match config {
            Config::Daemon(socket) => Self::Daemon(Daemon { socket }),
            Config::Fixture(fixture) => Self::Fixture(Box::new(Sim::new(*fixture))),
        }
    }
}

impl Daemon {
    /// A new connection, after the protocol handshake.
    pub(crate) async fn connect(&self) -> Result<IpcClient, ApiError> {
        IpcClient::connect(&self.socket).await.map_err(|error| {
            let code = if is_skew(&error) {
                ApiErrorCode::VersionSkew
            } else if matches!(&error, IpcError::Io(io) if io.kind() == io::ErrorKind::PermissionDenied)
            {
                ApiErrorCode::NotInGroup
            } else {
                ApiErrorCode::DaemonDown
            };
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, code, error.to_string())
        })
    }

    /// One request on a fresh connection.
    pub(crate) async fn ask(&self, request: Request) -> Result<Response, ApiError> {
        Ok(self.connect().await?.request(request).await?)
    }
}

/// Whether `error` says the daemon speaks another protocol version.
fn is_skew(error: &IpcError) -> bool {
    matches!(
        error,
        IpcError::VersionMismatch { .. }
            | IpcError::Remote {
                code: ErrorCode::VersionMismatch,
                ..
            }
    )
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
        Source::Fixture(sim) => runtime.spawn(fixture::replay(sim.read().clone(), live)),
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
    /// The daemon speaks another protocol version.
    Skew,
}

async fn follow_daemon(socket: PathBuf, live: Arc<Live>) {
    loop {
        let wait = match subscribe(&socket, &live).await {
            Ok(entries) => {
                run(&socket, &live, entries).await;
                RETRY
            }
            Err(Retry::Now) => continue,
            Err(Retry::Later) => RETRY,
            Err(Retry::Skew) => SKEW_RETRY,
        };
        live.down();
        tokio::time::sleep(wait).await;
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
    let client = IpcClient::connect(socket).await.map_err(|error| {
        if is_skew(&error) {
            Retry::Skew
        } else {
            Retry::Later
        }
    })?;
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
async fn run(socket: &Path, live: &Live, entries: Entries) {
    tokio::select! {
        () = apply(live, entries) => {}
        () = read_ticks(socket, live) => {}
    }
}

/// Applies entries until the journal subscription ends.
async fn apply(live: &Live, mut entries: Entries) {
    while let Some(Ok(entry)) = entries.recv().await {
        live.extend([entry]);
    }
}

/// Reads a tick every [`TICK`] until dropped.
///
/// Runs beside [`apply`], so a slow daemon delays no entry. A tick the daemon cannot
/// answer within [`TICK`] is skipped; the page notices the missing ticks and shows its
/// values as stale.
async fn read_ticks(socket: &Path, live: &Live) {
    let mut client: Option<IpcClient> = None;
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
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

#[cfg(test)]
#[cfg(unix)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use core::sync::atomic::{AtomicU32, Ordering};
    use std::os::unix::fs::PermissionsExt as _;

    use kith_ipc::{EntryKind, IpcServer};

    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn socket_path() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("kith-ui-ipc-{}-{n}.sock", std::process::id()))
    }

    fn entry(seq: u64) -> Entry {
        Entry {
            seq,
            at_ms: 1_000 + seq,
            kind: EntryKind::Warning {
                message: format!("w{seq}"),
            },
        }
    }

    async fn connect_code(socket: PathBuf) -> TestResult<ApiErrorCode> {
        match (Daemon { socket }).connect().await {
            Ok(_) => Err("connected".into()),
            Err(error) => Ok(error.code()),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_failures_map_to_what_the_page_shows() -> TestResult {
        assert_eq!(connect_code(socket_path()).await?, ApiErrorCode::DaemonDown);

        let skewed = socket_path();
        let server = IpcServer::bind(&skewed)?;
        tokio::spawn(async move {
            while let Ok(mut connection) = server.accept().await {
                let refused = Response::Error {
                    code: ErrorCode::VersionMismatch,
                    message: "upgrade".to_owned(),
                };
                let _sent = connection.respond(refused).await;
            }
        });
        assert_eq!(connect_code(skewed).await?, ApiErrorCode::VersionSkew);

        let locked = socket_path();
        let _server = IpcServer::bind(&locked)?;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))?;
        assert_eq!(connect_code(locked).await?, ApiErrorCode::NotInGroup);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_stops_at_the_entry_before_next() -> TestResult {
        let (sender, mut entries) = mpsc::channel(8);
        for seq in 3..6 {
            sender.send(Ok(entry(seq))).await?;
        }
        let read = history(&mut entries, Some(3), 5).await.ok_or("ended")?;
        assert_eq!(read.iter().map(|one| one.seq).collect::<Vec<_>>(), [3, 4]);
        assert_eq!(history(&mut entries, Some(9), 5).await, Some(Vec::new()));
        assert_eq!(history(&mut entries, None, 0).await, Some(Vec::new()));
        drop(sender);
        assert_eq!(history(&mut entries, None, 9).await, None);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_hung_tick_does_not_hold_back_entries() -> TestResult {
        let socket = socket_path();
        let server = IpcServer::bind(&socket)?;
        tokio::spawn(async move {
            while let Ok(mut connection) = server.accept().await {
                tokio::spawn(async move {
                    while let Ok(Some(request)) = connection.next_request().await {
                        if !matches!(request, Request::Journal { .. }) {
                            core::future::pending::<()>().await;
                        }
                        if connection
                            .respond(Response::Journaling { next: 0 })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        let _sent = connection.send_entry(entry(0)).await;
                        core::future::pending::<()>().await;
                    }
                });
            }
        });
        let live = Arc::new(Live::new("b0"));
        let _follower = follow(&Source::Daemon(Daemon { socket }), Arc::clone(&live))?;
        let arrived = tokio::time::timeout(TICK / 2, async {
            while live.last_entry().is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(arrived.is_ok(), "the entry waited for the tick");
        Ok(())
    }
}
