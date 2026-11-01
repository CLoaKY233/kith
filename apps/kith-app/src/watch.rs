//! Keeps the shell's model current: signs the tray's client in, reads the snapshot, then
//! follows the event stream, reading the snapshot again when the journal moves. When the
//! daemon does not answer it asks again every 2 s; when the stream ends it reopens it
//! with backoff.

use std::collections::BTreeMap;
use std::sync::Arc;

use kith_ui_core::wire::{ExitView, Tick};

use crate::client::{Client, Events, Failure};
use crate::model::{BootIn, Down, Reach, Snapshot};
use crate::shell::Shell;
use crate::timing::{DAEMON_POLL, RECONNECT_BACKOFF_MS, REFRESH_COALESCE};

/// The snapshot and every network's exits.
pub(crate) async fn load(client: &Client) -> Result<Snapshot, Failure> {
    let boot: BootIn = client.get("/api/boot").await?;
    let mut exits = BTreeMap::new();
    for network in &boot.status.networks {
        let path = format!("/api/networks/{}/exits", network.id);
        if let Ok(list) = client.get::<Vec<ExitView>>(&path).await {
            exits.insert(network.id.clone(), list);
        }
    }
    Ok(Snapshot { boot, exits })
}

fn reach(loaded: Result<Snapshot, Failure>) -> Reach {
    match loaded {
        Ok(snapshot) => Reach::Up(Box::new(snapshot)),
        Err(failure) => Reach::Down(Down::from_failure(&failure)),
    }
}

/// Runs for as long as the shell does.
pub(crate) async fn run(shell: Arc<Shell>) {
    let client = loop {
        match Client::start(shell.source()).await {
            Ok(client) => break client,
            Err(_) => tokio::time::sleep(DAEMON_POLL).await,
        }
    };
    shell.set_client(client.clone());
    let mut attempt = 0_usize;
    loop {
        let snapshot = match load(&client).await {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                shell.apply(Reach::Down(Down::from_failure(&failure)));
                shell.housekeeping();
                tokio::time::sleep(DAEMON_POLL).await;
                continue;
            }
        };
        let last = format!("{}:{}", snapshot.boot.boot, snapshot.boot.seq);
        shell.apply(Reach::Up(Box::new(snapshot)));
        if let Ok(mut events) = client.events(Some(&last)).await {
            attempt = 0;
            follow(&shell, &client, &mut events).await;
        }
        let wait = RECONNECT_BACKOFF_MS
            .get(attempt)
            .or(RECONNECT_BACKOFF_MS.last())
            .copied()
            .unwrap_or_default();
        tokio::time::sleep(core::time::Duration::from_millis(wait)).await;
        attempt = attempt.saturating_add(1);
    }
}

/// Follows one stream until it ends.
async fn follow(shell: &Arc<Shell>, client: &Client, events: &mut Events) {
    let mut due = false;
    loop {
        let next = if due {
            match tokio::time::timeout(REFRESH_COALESCE, events.next()).await {
                Ok(next) => next,
                Err(_quiet) => {
                    due = false;
                    shell.apply(reach(load(client).await));
                    continue;
                }
            }
        } else {
            events.next().await
        };
        let Some(event) = next else { return };
        match event.name.as_str() {
            "entry" | "gap" => due = true,
            "tick" => {
                if let Ok(tick) = serde_json::from_str::<Tick>(&event.data) {
                    shell.tick(&tick);
                }
            }
            "daemon_down" => {
                shell.apply(Reach::Down(Down::NotRunning));
                shell.housekeeping();
            }
            "daemon_up" | "reset" => {
                due = false;
                shell.apply(reach(load(client).await));
            }
            _ => {}
        }
    }
}
