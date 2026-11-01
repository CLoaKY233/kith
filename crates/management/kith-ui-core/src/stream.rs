//! `GET /api/events`: one server-sent event stream for everything live.
//!
//! - `event: entry`, `id: <boot>:<seq>`: a journal entry, flattened.
//! - `event: tick`, every 2 s, no id, never replayed: traffic counters and links. It is
//!   also the heartbeat, so there is no keep-alive comment.
//! - `event: reset`: the page's boot id is not the daemon's; refetch everything.
//! - `event: gap`: entries the page asked for have left the daemon's ring.
//! - `event: daemon_down` and `event: daemon_up`.
//!
//! The page resumes with `Last-Event-ID`, or `?last=<boot>:<seq>` on its first connect.

use core::convert::Infallible;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, Sse};
use futures_core::Stream;
use kith_ipc::{Entry, Request, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::api::ApiError;
use crate::ipc::{Daemon, Source};
use crate::wire::{Gap, LinkTick, Reset, Tick, TrafficTick};
use crate::{Shared, fixture, json, now_ms};

/// How often numbers are sent.
const TICK: Duration = Duration::from_secs(2);
/// Events a slow page may fall behind before the stream drops it.
const BUFFER: usize = 64;

/// The stream's events, fed by a task.
#[derive(Debug)]
pub(crate) struct Events(mpsc::Receiver<Event>);

impl Stream for Events {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx).map(|event| event.map(Ok))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Resume {
    /// `<boot>:<seq>` of the last entry the page applied.
    last: Option<String>,
}

/// `<boot>:<seq>` split, from `Last-Event-ID` or else `?last=`.
fn resume_point(headers: &HeaderMap, query: &Resume) -> Option<(String, u64)> {
    let text = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .or(query.last.as_deref())?;
    let (boot, seq) = text.trim().split_once(':')?;
    Some((boot.to_owned(), seq.parse().ok()?))
}

fn event(name: &str, data: &impl Serialize) -> Option<Event> {
    Event::default().event(name).json_data(data).ok()
}

/// `GET /api/events`.
pub(crate) async fn events(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<Resume>,
) -> Result<Sse<Events>, ApiError> {
    let resume = resume_point(&headers, &query);
    let stale = resume
        .as_ref()
        .is_some_and(|(boot, _)| *boot != shared.boot);
    let (sender, receiver) = mpsc::channel(BUFFER);
    if stale
        && let Some(reset) = event(
            "reset",
            &Reset {
                boot: shared.boot.clone(),
                started_at_ms: now_ms(),
            },
        )
    {
        let _full = sender.try_send(reset);
    }
    match &shared.source {
        Source::Fixture(recorded) => {
            let player = fixture::Player::new(recorded);
            tokio::spawn(play(player, sender));
        }
        Source::Daemon(daemon) => {
            let since = resume
                .filter(|_| !stale)
                .map(|(_, seq)| seq.saturating_add(1));
            let (_, entries) = daemon.connect().await?.journal(since).await?;
            tokio::spawn(forward(Arc::clone(&shared), since, entries, sender));
        }
    }
    Ok(Sse::new(Events(receiver)))
}

async fn play(mut player: fixture::Player, sender: mpsc::Sender<Event>) {
    let mut ticks = tokio::time::interval(TICK);
    loop {
        ticks.tick().await;
        let Some(tick) = event("tick", &player.tick()) else {
            continue;
        };
        if sender.send(tick).await.is_err() {
            return;
        }
    }
}

type Entries = mpsc::Receiver<Result<Entry, kith_ipc::IpcError>>;

async fn forward(
    shared: Arc<Shared>,
    mut expected: Option<u64>,
    mut entries: Entries,
    sender: mpsc::Sender<Event>,
) {
    let mut ticks = tokio::time::interval(TICK);
    let mut down = false;
    loop {
        let next = tokio::select! {
            entry = entries.recv() => match entry {
                Some(Ok(entry)) => {
                    if let Some(from) = expected.filter(|from| entry.seq > *from)
                        && let Some(gap) = event("gap", &Gap {
                            missed_from: from,
                            missed_to: entry.seq.saturating_sub(1),
                        })
                        && sender.send(gap).await.is_err()
                    {
                        return;
                    }
                    expected = Some(entry.seq.saturating_add(1));
                    json::entry(&entry).ok().and_then(|data| {
                        Event::default()
                            .event("entry")
                            .id(format!("{}:{}", shared.boot, entry.seq))
                            .json_data(data)
                            .ok()
                    })
                }
                Some(Err(_)) | None => event("daemon_down", &serde_json::json!({})),
            },
            _ = ticks.tick() => {
                let Source::Daemon(daemon) = &shared.source else { return };
                match tick(daemon).await {
                    Ok(tick) => {
                        let up = down.then(|| event("daemon_up", &serde_json::json!({}))).flatten();
                        down = false;
                        if let Some(up) = up && sender.send(up).await.is_err() {
                            return;
                        }
                        event("tick", &tick)
                    }
                    Err(_) if !down => {
                        down = true;
                        event("daemon_down", &serde_json::json!({}))
                    }
                    Err(_) => None,
                }
            }
        };
        if let Some(next) = next
            && sender.send(next).await.is_err()
        {
            return;
        }
    }
}

/// Counters and links, read from the daemon now.
async fn tick(daemon: &Daemon) -> Result<Tick, ApiError> {
    let mut client = daemon.connect().await?;
    let Response::Traffic(series) = client.request(Request::Traffic).await? else {
        return Err(ApiError::unexpected());
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
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let mut links = Vec::new();
    for network in &status.networks {
        let Response::Topology(reports) = client
            .request(Request::Topology {
                network: network.id,
            })
            .await?
        else {
            return Err(ApiError::unexpected());
        };
        for report in &reports {
            links.extend(report.links.iter().map(|link| LinkTick {
                network: network.id.to_string(),
                from: report.node.to_string(),
                to: link.peer.to_string(),
                path: link.path,
                relay: link.relay.map(|relay| relay.to_string()),
                rtt_ms: link.rtt_ms,
                loss: None,
                rate: Some(link.bytes_per_sec),
                rate_band: None,
                age_secs: report.age_secs,
            }));
        }
    }
    Ok(Tick {
        at_ms: now_ms(),
        traffic,
        links,
    })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn the_header_wins_over_the_query() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("b1:41"));
        let query = Resume {
            last: Some("b0:7".to_owned()),
        };
        assert_eq!(resume_point(&headers, &query), Some(("b1".to_owned(), 41)));
        assert_eq!(
            resume_point(&HeaderMap::new(), &query),
            Some(("b0".to_owned(), 7))
        );
        assert_eq!(
            resume_point(&HeaderMap::new(), &Resume { last: None }),
            None
        );
    }
}
