//! `GET /api/events`: one server-sent event stream for everything live, fanned out from
//! [`Live`] to every open page.
//!
//! - `event: entry`, `id: <boot>:<seq>`: a journal entry, flattened.
//! - `event: tick`, every 2 s, no id, never replayed: traffic counters and links. It is
//!   also the heartbeat, so there is no keep-alive comment.
//! - `event: reset`: the page's boot id is not the current one; refetch everything.
//! - `event: gap`: entries the page asked for are no longer kept.
//! - `event: daemon_down` and `event: daemon_up`.
//!
//! The page resumes with `Last-Event-ID`, or `?last=<boot>:<seq>` on its first connect.

use core::convert::Infallible;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, Sse};
use futures_core::Stream;
use kith_ipc::{Entry, TrafficSeries};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};

use crate::session::Token;
use crate::traffic::History;
use crate::wire::{Gap, Reset, Tick};
use crate::{Shared, json, now_ms};

/// Entries kept for pages that reconnect: as many as the daemon keeps.
const KEEP: usize = 4096;
/// Messages a page may fall behind before it catches up from the kept entries.
const LIVE_BUFFER: usize = 256;
/// Events queued for one page before its connection waits.
const BUFFER: usize = 64;
/// How long a snapshot or a new stream waits for the first word from the daemon.
const SETTLE: Duration = Duration::from_secs(2);

/// One server-sent event, serialized once however many pages receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    /// The `event:` field.
    name: &'static str,
    /// The `id:` field; only entries have one.
    id: Option<Arc<str>>,
    /// The `data:` field, compact JSON.
    data: Arc<str>,
}

impl Frame {
    fn new(name: &'static str, data: &impl Serialize) -> Option<Self> {
        Some(Self {
            name,
            id: None,
            data: Arc::from(serde_json::to_string(data).ok()?),
        })
    }

    fn signal(name: &'static str) -> Self {
        Self {
            name,
            id: None,
            data: Arc::from("{}"),
        }
    }

    fn entry(boot: &str, entry: &Entry) -> Option<Self> {
        let data = serde_json::to_string(&json::entry(entry).ok()?).ok()?;
        Some(Self {
            name: "entry",
            id: Some(Arc::from(format!("{boot}:{}", entry.seq))),
            data: Arc::from(data),
        })
    }

    fn event(&self) -> Event {
        let event = Event::default().event(self.name).data(&*self.data);
        match &self.id {
            Some(id) => event.id(&**id),
            None => event,
        }
    }
}

/// One message to every open stream.
#[derive(Debug, Clone)]
enum Message {
    /// A journal entry, with the boot and seq that its id names.
    Entry {
        boot: Arc<str>,
        seq: u64,
        frame: Frame,
    },
    /// A tick, reset, `daemon_down` or `daemon_up`.
    Signal(Frame),
}

/// Where a page is in the journal: the boot id and the last seq it applied.
type Cursor = (String, u64);

/// What the server knows of the daemon's current run.
#[derive(Debug)]
struct Run {
    /// Names the daemon's run; it changes when the daemon restarts.
    boot: Arc<str>,
    /// When the run started, in Unix milliseconds.
    started_at_ms: u64,
    /// The newest [`KEEP`] entries, oldest first.
    ring: VecDeque<Entry>,
    /// The seq the next entry gets.
    next: u64,
    /// Whether any run was synced; until then, the first sync keeps the initial boot id.
    synced: bool,
    /// Whether the daemon is unreachable.
    down: bool,
    /// The latest tick, for the boot snapshot.
    last_tick: Option<Tick>,
    /// Traffic per device over the last hour.
    traffic: History,
}

/// The live state every stream fans out from: the journal of the daemon's current run,
/// the latest tick and the traffic history. A follower in [`crate::ipc`] or
/// [`crate::fixture`] feeds it.
#[derive(Debug)]
pub(crate) struct Live {
    run: Mutex<Run>,
    messages: broadcast::Sender<Message>,
    settled: watch::Sender<bool>,
}

/// A new stream's first frames and the live messages that follow them.
#[derive(Debug)]
struct Opening {
    frames: Vec<Frame>,
    cursor: Cursor,
    messages: broadcast::Receiver<Message>,
}

/// A boot id nobody has seen: 16 hex digits.
fn fresh_boot() -> String {
    Token::random()
        .ok()
        .and_then(|token| token.hex().get(..16).map(str::to_owned))
        .unwrap_or_else(|| format!("{:016x}", now_ms()))
}

impl Live {
    /// Nothing known yet; `boot` names the first run.
    pub(crate) fn new(boot: &str) -> Self {
        let (messages, _) = broadcast::channel(LIVE_BUFFER);
        let (settled, _) = watch::channel(false);
        Self {
            run: Mutex::new(Run {
                boot: Arc::from(boot),
                started_at_ms: now_ms(),
                ring: VecDeque::new(),
                next: 0,
                synced: false,
                down: false,
                last_tick: None,
                traffic: History::default(),
            }),
            messages,
            settled,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Run> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends to every open stream. Called under the lock, so a stream never sees a
    /// message twice or out of order with its opening frames.
    fn send(&self, message: Message) {
        let _receivers = self.messages.send(message);
    }

    fn settle(&self) {
        self.settled.send_replace(true);
    }

    /// Waits until the first sync or the first failure, for at most [`SETTLE`].
    async fn settled(&self) {
        let mut settled = self.settled.subscribe();
        let _timeout = tokio::time::timeout(SETTLE, settled.wait_for(|done| *done)).await;
    }

    fn mark_up(&self, run: &mut Run) {
        if run.down {
            run.down = false;
            self.send(Message::Signal(Frame::signal("daemon_up")));
        }
    }

    /// The seq and time of the newest kept entry, to check on reconnect that the daemon
    /// is still on the same run.
    pub(crate) fn last_entry(&self) -> Option<(u64, u64)> {
        self.lock()
            .ring
            .back()
            .map(|entry| (entry.seq, entry.at_ms))
    }

    /// Drops the kept entries once the daemon is known to have restarted, so the next sync
    /// starts from the daemon's oldest entry and replaces the run.
    pub(crate) fn forget(&self) {
        self.lock().ring.clear();
    }

    /// Starts a run from its history and `next`, the seq its next entry gets. After the
    /// first run this is a restart: a new boot id, and every stream gets `reset`.
    pub(crate) fn restart(&self, history: Vec<Entry>, next: u64) {
        let mut run = self.lock();
        self.mark_up(&mut run);
        let started_at_ms = history
            .first()
            .filter(|entry| entry.seq == 0)
            .map_or_else(now_ms, |entry| entry.at_ms);
        run.started_at_ms = started_at_ms;
        run.ring = history.into_iter().collect();
        while run.ring.len() > KEEP {
            run.ring.pop_front();
        }
        run.next = run
            .ring
            .back()
            .map_or(next, |entry| next.max(entry.seq.saturating_add(1)));
        if run.synced {
            run.boot = Arc::from(fresh_boot());
            if let Some(reset) = Frame::new(
                "reset",
                &Reset {
                    boot: run.boot.to_string(),
                    started_at_ms,
                },
            ) {
                self.send(Message::Signal(reset));
            }
        }
        run.synced = true;
        drop(run);
        self.settle();
    }

    /// Adds entries of the current run and sends the new ones; entries already kept are
    /// skipped, so a reconnect can pass its whole history.
    pub(crate) fn extend(&self, entries: impl IntoIterator<Item = Entry>) {
        let mut run = self.lock();
        self.mark_up(&mut run);
        for entry in entries {
            if entry.seq < run.next {
                continue;
            }
            run.next = entry.seq.saturating_add(1);
            if let Some(frame) = Frame::entry(&run.boot, &entry) {
                self.send(Message::Entry {
                    boot: Arc::clone(&run.boot),
                    seq: entry.seq,
                    frame,
                });
            }
            if run.ring.len() == KEEP {
                run.ring.pop_front();
            }
            run.ring.push_back(entry);
        }
        drop(run);
        self.settle();
    }

    /// The daemon stopped answering. Sent once until it answers again.
    pub(crate) fn down(&self) {
        let mut run = self.lock();
        if !run.down {
            run.down = true;
            self.send(Message::Signal(Frame::signal("daemon_down")));
        }
        drop(run);
        self.settle();
    }

    /// Sends a tick and keeps it for the next snapshot.
    pub(crate) fn tick(&self, tick: Tick) {
        let mut run = self.lock();
        if let Some(frame) = Frame::new("tick", &tick) {
            self.send(Message::Signal(frame));
        }
        run.last_tick = Some(tick);
    }

    /// Changes the traffic history.
    pub(crate) fn record(&self, change: impl FnOnce(&mut History)) {
        change(&mut self.lock().traffic);
    }

    /// The boot id and the last seq applied, read before the rest of a snapshot so that
    /// no entry falls between the two.
    pub(crate) async fn position(&self) -> (String, u64) {
        self.settled().await;
        let run = self.lock();
        (run.boot.to_string(), run.next.saturating_sub(1))
    }

    /// Every device's newest `count` traffic samples; empty before the first tick.
    pub(crate) fn traffic(&self, count: usize) -> Vec<TrafficSeries> {
        self.lock().traffic.series(count)
    }

    /// The latest tick, if one was read.
    pub(crate) fn last_tick(&self) -> Option<Tick> {
        self.lock().last_tick.clone()
    }

    /// The frames a stream opens with, given where the page left off, and the live
    /// messages after them.
    fn open(&self, resume: Option<Cursor>) -> Opening {
        let run = self.lock();
        let messages = self.messages.subscribe();
        let latest = run.next.saturating_sub(1);
        let mut frames = Vec::new();
        match resume {
            None => {}
            Some((boot, seq)) if *boot != *run.boot || seq > latest => {
                frames.extend(Frame::new(
                    "reset",
                    &Reset {
                        boot: run.boot.to_string(),
                        started_at_ms: run.started_at_ms,
                    },
                ));
            }
            Some((_, seq)) => {
                let from = seq.saturating_add(1);
                let oldest = run.ring.front().map_or(run.next, |entry| entry.seq);
                if from < oldest {
                    frames.extend(Frame::new(
                        "gap",
                        &Gap {
                            missed_from: from,
                            missed_to: oldest.saturating_sub(1),
                        },
                    ));
                }
                frames.extend(
                    run.ring
                        .iter()
                        .filter(|entry| entry.seq >= from)
                        .filter_map(|entry| Frame::entry(&run.boot, entry)),
                );
            }
        }
        if run.down {
            frames.push(Frame::signal("daemon_down"));
        }
        Opening {
            frames,
            cursor: (run.boot.to_string(), latest),
            messages,
        }
    }
}

/// The stream's events, fed by [`relay`].
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
fn resume_point(headers: &HeaderMap, query: &Resume) -> Option<Cursor> {
    let text = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .or(query.last.as_deref())?;
    let (boot, seq) = text.trim().split_once(':')?;
    if boot.is_empty() {
        return None;
    }
    Some((boot.to_owned(), seq.parse().ok()?))
}

/// `GET /api/events`.
pub(crate) async fn events(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<Resume>,
) -> Sse<Events> {
    let resume = resume_point(&headers, &query);
    shared.live.settled().await;
    let (sender, receiver) = mpsc::channel(BUFFER);
    tokio::spawn(relay(Arc::clone(&shared.live), resume, sender));
    Sse::new(Events(receiver))
}

/// Feeds one page: its opening frames, then everything live. A page too slow for the
/// live messages catches up from the kept entries; it stops when the page goes away.
async fn relay(live: Arc<Live>, resume: Option<Cursor>, sender: mpsc::Sender<Event>) {
    let mut opening = live.open(resume);
    loop {
        for frame in opening.frames.drain(..) {
            if sender.send(frame.event()).await.is_err() {
                return;
            }
        }
        let lagged = loop {
            let message = tokio::select! {
                () = sender.closed() => return,
                message = opening.messages.recv() => message,
            };
            let frame = match message {
                Ok(Message::Entry { boot, seq, frame }) => {
                    opening.cursor = (boot.to_string(), seq);
                    frame
                }
                Ok(Message::Signal(frame)) => frame,
                Err(RecvError::Lagged(_)) => break opening.cursor,
                Err(RecvError::Closed) => return,
            };
            if sender.send(frame.event()).await.is_err() {
                return;
            }
        };
        opening = live.open(Some(lagged));
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use kith_ipc::EntryKind;

    use super::*;

    fn entry(seq: u64) -> Entry {
        Entry {
            seq,
            at_ms: 1_000 + seq,
            kind: EntryKind::Log {
                level: "info".into(),
                target: "test".into(),
                message: format!("entry {seq}"),
            },
        }
    }

    /// Each opening frame as `name` or `name id`.
    fn shape(opening: &Opening) -> Vec<String> {
        opening
            .frames
            .iter()
            .map(|frame| match &frame.id {
                Some(id) => format!("{} {id}", frame.name),
                None => frame.name.to_owned(),
            })
            .collect()
    }

    /// Everything sent to `messages` so far, as `name` or `entry <seq>`.
    fn sent(messages: &mut broadcast::Receiver<Message>) -> Vec<String> {
        core::iter::from_fn(|| messages.try_recv().ok())
            .map(|message| match message {
                Message::Entry { seq, .. } => format!("entry {seq}"),
                Message::Signal(frame) => frame.name.to_owned(),
            })
            .collect()
    }

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
        for bad in ["", "b0", ":4", "b0:x", "b0:-1"] {
            let query = Resume {
                last: Some(bad.to_owned()),
            };
            assert_eq!(resume_point(&HeaderMap::new(), &query), None, "{bad}");
        }
    }

    #[test]
    fn a_page_resumes_after_its_last_entry() {
        let live = Live::new("b0");
        live.restart((0..5).map(entry).collect(), 5);
        let opening = live.open(Some(("b0".to_owned(), 2)));
        assert_eq!(shape(&opening), ["entry b0:3", "entry b0:4"]);
        assert_eq!(opening.cursor, ("b0".to_owned(), 4));
        assert_eq!(shape(&live.open(Some(("b0".to_owned(), 4)))), [""; 0]);
        assert_eq!(shape(&live.open(None)), [""; 0]);
    }

    #[test]
    fn a_page_from_another_run_or_ahead_of_this_one_is_reset() {
        let live = Live::new("b0");
        live.restart((0..5).map(entry).collect(), 5);
        assert_eq!(shape(&live.open(Some(("old".to_owned(), 2)))), ["reset"]);
        assert_eq!(shape(&live.open(Some(("b0".to_owned(), 9)))), ["reset"]);
    }

    #[test]
    fn entries_that_left_the_ring_are_reported_as_a_gap() {
        let live = Live::new("b0");
        live.restart((100..103).map(entry).collect(), 103);
        let opening = live.open(Some(("b0".to_owned(), 50)));
        assert_eq!(
            shape(&opening),
            ["gap", "entry b0:100", "entry b0:101", "entry b0:102"]
        );
        assert_eq!(
            opening.frames.first().map(|frame| &*frame.data),
            Some(r#"{"missed_from":51,"missed_to":99}"#)
        );
    }

    #[test]
    fn a_restart_after_the_first_run_changes_the_boot_and_resets_open_streams() {
        let live = Live::new("b0");
        live.restart((0..3).map(entry).collect(), 3);
        let mut messages = live.open(None).messages;
        live.down();
        live.down();
        live.restart(vec![entry(0)], 1);
        assert_eq!(sent(&mut messages), ["daemon_down", "daemon_up", "reset"]);
        let opening = live.open(Some(("b0".to_owned(), 2)));
        assert_eq!(shape(&opening), ["reset"]);
        assert_ne!(opening.cursor.0, "b0");
    }

    #[test]
    fn kept_entries_are_not_sent_twice() {
        let live = Live::new("b0");
        live.restart((0..3).map(entry).collect(), 3);
        let mut messages = live.open(None).messages;
        live.extend((1..5).map(entry));
        assert_eq!(sent(&mut messages), ["entry 3", "entry 4"]);
        assert_eq!(live.last_entry(), Some((4, 1_004)));
    }

    #[test]
    fn a_stream_opened_while_the_daemon_is_down_says_so() {
        let live = Live::new("b0");
        live.down();
        assert_eq!(shape(&live.open(None)), ["daemon_down"]);
    }
}
