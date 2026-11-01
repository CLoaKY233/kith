//! `GET /api/status`, `GET /api/pending` and `GET /api/boot`: the snapshot the page
//! renders its first frame from.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use kith_ipc::{BlockedInfo, PendingJoin, Request, Response, Status, TrafficSeries};
use serde_json::json;

use crate::api::{Answer, ApiError};
use crate::ipc::{Daemon, Source};
use crate::wire::{
    BlockedGroup, Boot, Claimed, Host, JoinRequest, Membership, Pending, Role, Seen, StatusView,
    Versions,
};
use crate::{Shared, json, now_ms};

/// Samples per device in the boot snapshot: five minutes at one every two seconds.
const TAIL: usize = 150;
/// How long a join request waits for an admin before it lapses.
const JOIN_LIFETIME_MS: u64 = 10 * 60 * 1000;
/// Crockford base32, for check codes.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// `GET /api/status`.
pub(crate) async fn status(State(shared): State<Arc<Shared>>) -> Answer {
    let view = match &shared.source {
        Source::Fixture(fixture) => fixture.status.clone(),
        Source::Daemon(daemon) => status_view(daemon).await?,
    };
    Ok(Json(json!(view)))
}

/// `GET /api/pending`.
pub(crate) async fn pending(State(shared): State<Arc<Shared>>) -> Answer {
    let pending = match &shared.source {
        Source::Fixture(fixture) => fixture.pending.clone(),
        Source::Daemon(daemon) => {
            let Response::Status(status) = daemon.ask(Request::Status).await? else {
                return Err(ApiError::unexpected());
            };
            daemon_pending(daemon, &status).await?
        }
    };
    Ok(Json(json!(pending)))
}

/// `GET /api/boot`: what `index.html` inlines, for a page that started without it.
pub(crate) async fn boot(State(shared): State<Arc<Shared>>) -> Answer {
    Ok(Json(json!(snapshot(&shared).await?)))
}

/// The boot snapshot.
pub(crate) async fn snapshot(shared: &Shared) -> Result<Boot, ApiError> {
    match &shared.source {
        Source::Fixture(fixture) => Ok(crate::fixture::boot(fixture)),
        Source::Daemon(daemon) => daemon_boot(shared, daemon).await,
    }
}

fn versions(daemon: &str) -> Versions {
    Versions {
        ui: env!("CARGO_PKG_VERSION").to_owned(),
        daemon: daemon.to_owned(),
        compatible: true,
    }
}

fn host() -> Host {
    let name = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_default();
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    };
    Host {
        name,
        os: os.to_owned(),
    }
}

fn view(status: &Status, daemon_version: &str) -> Result<StatusView, ApiError> {
    let memberships = status
        .networks
        .iter()
        .map(|network| Membership {
            network: network.id.to_string(),
            name: String::new(),
            address: String::new(),
            role: if network.admin {
                Role::Admin
            } else {
                Role::Member
            },
            key_fpr: String::new(),
            made_admin_by: None,
            made_admin_at_ms: None,
            admins: Vec::new(),
            share_links: true,
        })
        .collect();
    Ok(StatusView {
        status: json::to_value(status)?,
        operator: true,
        versions: versions(daemon_version),
        paused_until: None,
        memberships,
        host: host(),
        exit_state: None,
    })
}

async fn status_view(daemon: &Daemon) -> Result<StatusView, ApiError> {
    let mut client = daemon.connect().await?;
    let version = client.daemon_version().to_owned();
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    view(&status, &version)
}

/// A stand-in check code from the device id until the daemon sends its own.
fn check_code(node: &str) -> String {
    node.bytes()
        .take(6)
        .filter_map(|byte| CROCKFORD.get(usize::from(byte % 32)))
        .map(|&byte| char::from(byte))
        .collect()
}

fn join_request(join: &PendingJoin, now: u64) -> JoinRequest {
    let network = join.network.to_string();
    let node = join.node.to_string();
    let requested = now.saturating_sub(join.age_secs.saturating_mul(1000));
    JoinRequest {
        id: format!("join:{network}:{node}"),
        check_code: check_code(&node),
        claimed: Claimed {
            name: join.name.clone(),
            os: None,
        },
        seen: Seen {
            ip: join.from.clone(),
            key_fpr: node.get(..8).unwrap_or_default().to_owned(),
        },
        requested_at_ms: requested,
        expires_at_ms: requested.saturating_add(JOIN_LIFETIME_MS),
        network,
        node,
    }
}

fn blocked_group(blocked: &BlockedInfo) -> BlockedGroup {
    let network = blocked.network.to_string();
    let from = blocked.from.to_string();
    let to = blocked.to.to_string();
    let port = blocked
        .port
        .map(|port| port.to_string())
        .unwrap_or_default();
    BlockedGroup {
        id: format!("blocked:{network}:{from}:{to}:{}:{port}", blocked.protocol),
        network,
        from,
        from_name: blocked.from_name.clone(),
        to,
        to_name: blocked.to_name.clone(),
        protocol: blocked.protocol.clone(),
        port: blocked.port,
        count: blocked.count,
        first_ms: blocked.last_ms,
        last_ms: blocked.last_ms,
        rule: blocked.rule.clone(),
    }
}

async fn daemon_pending(daemon: &Daemon, status: &Status) -> Result<Pending, ApiError> {
    let now = now_ms();
    let Response::PendingJoins(joins) = daemon.ask(Request::PendingJoins).await? else {
        return Err(ApiError::unexpected());
    };
    let mut blocked = Vec::new();
    for network in &status.networks {
        let Response::Blocked(list) = daemon
            .ask(Request::Blocked {
                network: network.id,
            })
            .await?
        else {
            return Err(ApiError::unexpected());
        };
        blocked.extend(list.iter().map(blocked_group));
    }
    Ok(Pending {
        joins: joins.iter().map(|join| join_request(join, now)).collect(),
        blocked,
    })
}

fn tail(mut series: Vec<TrafficSeries>) -> Vec<TrafficSeries> {
    for one in &mut series {
        let keep = one.samples.len().saturating_sub(TAIL);
        one.samples.drain(..keep);
    }
    series
}

async fn daemon_boot(shared: &Shared, daemon: &Daemon) -> Result<Boot, ApiError> {
    let mut client = daemon.connect().await?;
    let version = client.daemon_version().to_owned();
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let mut peers = BTreeMap::new();
    for network in &status.networks {
        let Response::Peers(list) = client
            .request(Request::Peers {
                network: network.id,
            })
            .await?
        else {
            return Err(ApiError::unexpected());
        };
        peers.insert(network.id.to_string(), json::to_value(&list)?);
    }
    let Response::Traffic(traffic) = client.request(Request::Traffic).await? else {
        return Err(ApiError::unexpected());
    };
    let pending = daemon_pending(daemon, &status).await?;
    let (next, _entries) = daemon.connect().await?.journal(None).await?;
    let view = view(&status, &version)?;
    Ok(Boot {
        boot: shared.boot.clone(),
        seq: next.saturating_sub(1),
        server_time_ms: now_ms(),
        versions: view.versions.clone(),
        status: view,
        peers,
        traffic_tail: json::to_value(&tail(traffic))?,
        pending,
        last_tick: None,
    })
}
