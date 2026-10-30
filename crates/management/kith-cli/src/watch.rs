//! `kith watch`: the current state, then each change as it happens.
//!
//! The daemon's event stream says little (a handshake, a ledger change), so the state is
//! polled and compared; only warnings come from the stream.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use kith_ipc::{Event, IpcClient, PeerInfo, Status};

use crate::exec::{CliError, connect, fetch_peers, fetch_status};
use crate::render;

const POLL: Duration = Duration::from_secs(2);

/// What `kith watch` compares from one poll to the next.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    /// The exit in use, as `network/device`.
    exit: Option<String>,
    /// Each peer, keyed by network name and peer name.
    peers: BTreeMap<(String, String), PeerInfo>,
    /// Packets dropped so far, by reason.
    drops: BTreeMap<String, u64>,
}

impl Snapshot {
    pub(crate) fn new(status: &Status, peers: &[Vec<PeerInfo>]) -> Self {
        let peers = status
            .networks
            .iter()
            .zip(peers)
            .flat_map(|(network, peers)| {
                peers
                    .iter()
                    .map(|peer| ((network.name.clone(), peer.name.clone()), peer.clone()))
            })
            .collect();
        Self {
            exit: status.exit_node.clone(),
            peers,
            drops: status.drops.clone(),
        }
    }
}

/// What changed from `before` to `after`, one line each.
pub(crate) fn changes(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut lines = Vec::new();
    for ((network, name), peer) in &after.peers {
        let who = format!("{network}/{name}");
        let Some(old) = before.peers.get(&(network.clone(), name.clone())) else {
            lines.push(format!("{who} joined ({})", peer.address));
            continue;
        };
        let path = render::path_cell(peer);
        match (old.online, peer.online) {
            (false, true) => lines.push(format!("{who} online, {path}")),
            (true, false) => lines.push(format!("{who} offline")),
            (true, true) if render::path_cell(old) != path => {
                lines.push(format!("{who} now {path}"));
            }
            (true, true) if old.endpoint.is_some() && peer.endpoint != old.endpoint => {
                let endpoint = peer.endpoint.as_deref().unwrap_or("-");
                lines.push(format!("{who} moved to {endpoint}"));
            }
            _ => {}
        }
    }
    for (network, name) in before.peers.keys() {
        if !after.peers.contains_key(&(network.clone(), name.clone())) {
            lines.push(format!("{network}/{name} removed"));
        }
    }
    if before.exit != after.exit {
        lines.push(match &after.exit {
            Some(exit) => format!("internet traffic now goes through {exit}"),
            None => "internet traffic no longer goes through an exit".to_owned(),
        });
    }
    for (reason, count) in &after.drops {
        let new = count.saturating_sub(before.drops.get(reason).copied().unwrap_or(0));
        if new > 0 {
            lines.push(format!("{new} packets dropped: {reason}"));
        }
    }
    lines
}

/// Prints the state, then every change until the daemon stops.
pub(crate) async fn run(socket: &Path, out: &mut impl Write) -> Result<(), CliError> {
    let mut poller = connect(socket).await?;
    let mut events = connect(socket).await?.watch().await?;
    let mut before = snapshot(&mut poller, Some(out)).await?;
    writeln!(out, "\nwatching for changes; ctrl-c to stop")?;
    out.flush()?;
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let Ok(after) = snapshot(&mut poller, None::<&mut Vec<u8>>).await else {
                    writeln!(out, "{} kith stopped", stamp(started.elapsed()))?;
                    return Ok(());
                };
                for line in changes(&before, &after) {
                    writeln!(out, "{} {line}", stamp(started.elapsed()))?;
                }
                out.flush()?;
                before = after;
            }
            event = events.recv() => match event {
                Some(Ok(Event::Warning { message })) => {
                    writeln!(out, "{} warning: {message}", stamp(started.elapsed()))?;
                    out.flush()?;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
                None => {
                    writeln!(out, "{} kith stopped", stamp(started.elapsed()))?;
                    return Ok(());
                }
            },
        }
    }
}

/// Reads the state, printing it as `kith status` does when `print` is given.
async fn snapshot(
    client: &mut IpcClient,
    print: Option<&mut impl Write>,
) -> Result<Snapshot, CliError> {
    let status = fetch_status(client).await?;
    let mut peers = Vec::with_capacity(status.networks.len());
    for network in &status.networks {
        peers.push(fetch_peers(client, network.id).await?);
    }
    if let Some(out) = print {
        render::status(out, &status, &peers)?;
    }
    Ok(Snapshot::new(&status, &peers))
}

/// Time since watching began, as `[mm:ss]` or `[h:mm:ss]`.
fn stamp(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("[{hours}:{minutes:02}:{seconds:02}]")
    } else {
        format!("[{minutes:02}:{seconds:02}]")
    }
}
