//! `GET /api/status`, `GET /api/pending` and `GET /api/boot`: the snapshot the page
//! renders its first frame from.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use kith_ipc::{
    BlockedInfo, DnsRecord, IpcClient, IpcError, LinkReport, NetworkSummary, PeerInfo, PendingJoin,
    RecordInfo, Request, Response, Status, TrafficSeries,
};
use kith_types::NodeId;
use serde_json::{Value, json};

use crate::api::codes::{check_code, key_fpr};
use crate::api::{Answer, ApiError, network_id};
use crate::ipc::{Daemon, Source};
use crate::wire::{
    BlockedGroup, Boot, Claimed, ConsoleMode, ExitState, Fixture, Host, JoinRequest, LinkTick, Me,
    Membership, Pending, RateBand, Role, Seen, StatusView, Tick, TrafficTick, Versions,
};
use crate::{Shared, json, now_ms};

/// Samples per device in the boot snapshot: five minutes at one every two seconds.
const TAIL: usize = 150;
/// How long a join request waits for an admin before it lapses, as the daemon keeps it.
pub(crate) const JOIN_LIFETIME_MS: u64 = 10 * 60 * 1000;
/// Below this many bytes per second a link is idle.
const LIGHT_FROM: u64 = 1000;
/// Above this many bytes per second a link is busy.
const BUSY_ABOVE: u64 = 1_000_000;

/// `GET /api/status`.
///
/// Without a session (only reachable in public mode) it says just `{mode, me: null}`, so
/// the signed-out page knows to offer passkey sign-in.
pub(crate) async fn status(
    State(shared): State<Arc<Shared>>,
    axum::extract::Extension(crate::session::Caller(principal)): axum::extract::Extension<
        crate::session::Caller,
    >,
) -> Answer {
    let mode = console_mode(&shared);
    let Some(principal) = principal else {
        return Ok(Json(json!({ "mode": mode, "me": null })));
    };
    let mut view = match &shared.source {
        Source::Fixture(sim) => sim.read().status.clone(),
        Source::Daemon(daemon) => read(&shared, daemon, false).await?.view,
    };
    view.mode = mode;
    view.me = me(&shared, Some(&principal)).await?;
    Ok(Json(json!(view)))
}

/// The signed-in person, as `me` in the status; `None` for the local link's session.
pub(crate) async fn me(
    shared: &Shared,
    principal: Option<&crate::console::Principal>,
) -> Result<Option<Me>, ApiError> {
    if principal
        .and_then(crate::console::Principal::account)
        .is_none()
    {
        return Ok(None);
    }
    let (_, person) = crate::console::signed_in(shared, principal).await?;
    Ok(Some(Me {
        id: person.id,
        name: person.name,
        role: person.role,
    }))
}

/// How this server is reached, as the page reads it.
pub(crate) const fn console_mode(shared: &Shared) -> ConsoleMode {
    match shared.mode {
        crate::console::Mode::Local => ConsoleMode::Local,
        crate::console::Mode::Public { .. } => ConsoleMode::Public,
    }
}

/// `GET /api/pending`.
pub(crate) async fn pending(State(shared): State<Arc<Shared>>) -> Answer {
    let pending = match &shared.source {
        Source::Fixture(sim) => sim.read().pending.clone(),
        Source::Daemon(daemon) => {
            let mut client = daemon.connect().await?;
            let Response::Status(status) = client.request(Request::Status).await? else {
                return Err(ApiError::unexpected());
            };
            daemon_pending(&mut client, &status).await?
        }
    };
    Ok(Json(json!(pending)))
}

/// `GET /api/boot`: what `index.html` inlines, for a page that started without it.
pub(crate) async fn boot(
    State(shared): State<Arc<Shared>>,
    axum::extract::Extension(crate::session::Caller(principal)): axum::extract::Extension<
        crate::session::Caller,
    >,
) -> Answer {
    Ok(Json(json!(
        crate::ipc::snapshot(&shared, principal.as_ref()).await?
    )))
}

/// The boot snapshot as the source has it; [`crate::ipc::snapshot`] puts the current
/// `boot`, `seq`, traffic and tick over it.
pub(crate) async fn snapshot(shared: &Shared) -> Result<Boot, ApiError> {
    match &shared.source {
        Source::Fixture(sim) => Ok(crate::fixture::boot(&sim.read())),
        Source::Daemon(daemon) => daemon_boot(shared, daemon).await,
    }
}

/// The recorded network `net` names: 400 when it is not a network id, 404 when the
/// recording does not hold it, as the daemon would answer.
pub(crate) fn recorded_network<'fixture>(
    fixture: &'fixture Fixture,
    net: &str,
) -> Result<&'fixture Value, ApiError> {
    network_id(net)?;
    fixture
        .status
        .status
        .get("networks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|network| network.get("id").and_then(Value::as_str) == Some(net))
        .ok_or_else(|| ApiError::not_found("this device is not in that network"))
}

/// The computer's name, as `hostname` gives it without the domain, and its OS.
pub(crate) fn host() -> Host {
    let name = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| Some(text.trim().split('.').next()?.to_owned()))
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

/// This device as one network sees it.
#[derive(Debug, Default)]
pub(crate) struct Local {
    /// Its name in the network.
    pub(crate) name: String,
    /// Its mesh IPv4 address.
    pub(crate) address: String,
}

/// One network, as read for a snapshot.
#[derive(Debug)]
struct Read {
    summary: NetworkSummary,
    local: Local,
    reports: Vec<LinkReport>,
    peers: Option<Vec<PeerInfo>>,
}

/// Everything a snapshot needs from the daemon, read on one connection.
#[derive(Debug)]
struct Snapshot {
    view: StatusView,
    status: Status,
    networks: Vec<Read>,
}

/// An answer the daemon may refuse, such as a topology it cannot measure yet; a refusal
/// reads as nothing rather than failing the whole snapshot.
fn optional<T>(
    answer: Result<Response, IpcError>,
    pick: fn(Response) -> Option<T>,
) -> Result<Option<T>, ApiError> {
    match answer {
        Ok(response) => pick(response).map(Some).ok_or_else(ApiError::unexpected),
        Err(IpcError::Remote { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// The suffix of every name in a network, such as `home.kith.internal`.
pub(crate) fn suffix(network_name: &str) -> String {
    format!("{network_name}.kith.internal")
}

/// This device's name and address in `summary`'s network.
pub(crate) async fn local(
    client: &mut IpcClient,
    summary: &NetworkSummary,
    node: NodeId,
) -> Result<(Local, Vec<LinkReport>), ApiError> {
    let network = summary.id;
    let reports = optional(client.request(Request::Topology { network }).await, |r| {
        if let Response::Topology(reports) = r {
            Some(reports)
        } else {
            None
        }
    })?
    .unwrap_or_default();
    let name = reports
        .iter()
        .find(|report| report.node == node)
        .map(|report| report.name.clone())
        .unwrap_or_default();
    let records = optional(client.request(Request::Records { network }).await, |r| {
        if let Response::Records(records) = r {
            Some(records)
        } else {
            None
        }
    })?
    .unwrap_or_default();
    let full = format!("{name}.{}", suffix(&summary.name));
    let address = records
        .iter()
        .filter(|record| record.device && record.name == full)
        .find_map(|record: &RecordInfo| {
            if let DnsRecord::A(address) = &record.record {
                Some(address.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    Ok((Local { name, address }, reports))
}

async fn peers_of(
    client: &mut IpcClient,
    summary: &NetworkSummary,
) -> Result<Vec<PeerInfo>, ApiError> {
    let Response::Peers(peers) = client
        .request(Request::Peers {
            network: summary.id,
        })
        .await?
    else {
        return Err(ApiError::unexpected());
    };
    Ok(peers)
}

/// How the exit in use is doing: an offline exit keeps its routes, so traffic stops.
fn exit_state(status: &Status, networks: &[Read]) -> Option<ExitState> {
    let (network_name, device) = status.exit_node.as_deref()?.split_once('/')?;
    let online = networks
        .iter()
        .find(|read| read.summary.name == network_name)?
        .peers
        .as_ref()?
        .iter()
        .find(|peer| peer.name == device)?
        .online;
    Some(if online {
        ExitState::Active
    } else {
        ExitState::OfflineBlocked
    })
}

fn membership(read: &Read, key: Option<&NodeId>) -> Membership {
    let admin = read.summary.admin;
    Membership {
        network: read.summary.id.to_string(),
        name: read.local.name.clone(),
        address: read.local.address.clone(),
        role: if admin { Role::Admin } else { Role::Member },
        key_fpr: key.map(key_fpr).unwrap_or_default(),
        made_admin_by: None,
        made_admin_at_ms: None,
        admins: if admin && !read.local.name.is_empty() {
            vec![read.local.name.clone()]
        } else {
            Vec::new()
        },
        share_links: true,
    }
}

/// Reads the status and each network on one connection; with `all_peers`, every
/// network's devices too, else only those of the exit's network.
async fn read(shared: &Shared, daemon: &Daemon, all_peers: bool) -> Result<Snapshot, ApiError> {
    let mut client = daemon.connect().await?;
    let version = client.daemon_version().to_owned();
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let exit_network = status
        .exit_node
        .as_deref()
        .and_then(|exit| exit.split_once('/'))
        .map(|(network, _)| network.to_owned());
    let mut networks = Vec::with_capacity(status.networks.len());
    for summary in &status.networks {
        let (local, reports) = match status.node {
            Some(node) => local(&mut client, summary, node).await?,
            None => (Local::default(), Vec::new()),
        };
        let peers = if all_peers || exit_network.as_deref() == Some(summary.name.as_str()) {
            Some(peers_of(&mut client, summary).await?)
        } else {
            None
        };
        networks.push(Read {
            summary: summary.clone(),
            local,
            reports,
            peers,
        });
    }
    let view = StatusView {
        status: json::to_value(&status)?,
        operator: true,
        versions: Versions {
            ui: env!("CARGO_PKG_VERSION").to_owned(),
            daemon: version,
            compatible: true,
        },
        paused_until: None,
        memberships: networks
            .iter()
            .map(|read| membership(read, status.node.as_ref()))
            .collect(),
        host: shared.api.host.clone(),
        exit_state: exit_state(&status, &networks),
        mode: ConsoleMode::default(),
        me: None,
    };
    Ok(Snapshot {
        view,
        status,
        networks,
    })
}

fn join_request(join: &PendingJoin, now: u64) -> JoinRequest {
    let network = join.network.to_string();
    let node = join.node.to_string();
    let requested = now.saturating_sub(join.age_secs.saturating_mul(1000));
    JoinRequest {
        id: format!("join:{network}:{node}"),
        check_code: check_code(&join.node),
        claimed: Claimed {
            name: join.name.clone(),
            os: None,
        },
        seen: Seen {
            ip: join.from.clone(),
            key_fpr: key_fpr(&join.node),
        },
        requested_at_ms: requested,
        expires_at_ms: requested.saturating_add(JOIN_LIFETIME_MS),
        network,
        node,
    }
}

/// The id of a blocked group: everything that tells one refused connection from another.
pub(crate) fn blocked_id(blocked: &BlockedInfo) -> String {
    let port = blocked
        .port
        .map(|port| port.to_string())
        .unwrap_or_default();
    format!(
        "blocked:{}:{}:{}:{}:{port}",
        blocked.network, blocked.from, blocked.to, blocked.protocol
    )
}

fn blocked_group(blocked: &BlockedInfo) -> BlockedGroup {
    BlockedGroup {
        id: blocked_id(blocked),
        network: blocked.network.to_string(),
        from: blocked.from.to_string(),
        from_name: blocked.from_name.clone(),
        to: blocked.to.to_string(),
        to_name: blocked.to_name.clone(),
        protocol: blocked.protocol.clone(),
        port: blocked.port,
        count: blocked.count,
        first_ms: blocked.last_ms,
        last_ms: blocked.last_ms,
        rule: blocked.rule.clone(),
    }
}

/// Every join request and blocked connection in this device's networks.
async fn daemon_pending(client: &mut IpcClient, status: &Status) -> Result<Pending, ApiError> {
    let now = now_ms();
    let Response::PendingJoins(joins) = client.request(Request::PendingJoins).await? else {
        return Err(ApiError::unexpected());
    };
    let mut blocked = Vec::new();
    for summary in &status.networks {
        let list = optional(
            client
                .request(Request::Blocked {
                    network: summary.id,
                })
                .await,
            |r| {
                if let Response::Blocked(list) = r {
                    Some(list)
                } else {
                    None
                }
            },
        )?
        .unwrap_or_default();
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

/// How busy a link is, for members, who never see another device's exact rate.
pub(crate) const fn rate_band(bytes_per_sec: u64) -> RateBand {
    if bytes_per_sec < LIGHT_FROM {
        RateBand::Idle
    } else if bytes_per_sec <= BUSY_ABOVE {
        RateBand::Light
    } else {
        RateBand::Busy
    }
}

/// Each reported link of a network. Admins and this device's own links get the exact
/// rate; a member sees only the band of links between other devices.
pub(crate) fn links(
    summary: &NetworkSummary,
    reports: &[LinkReport],
    me: Option<NodeId>,
) -> Vec<LinkTick> {
    let mut links = Vec::new();
    for report in reports {
        links.extend(report.links.iter().map(|link| {
            let exact = summary.admin || Some(report.node) == me || Some(link.peer) == me;
            LinkTick {
                network: summary.id.to_string(),
                from: report.node.to_string(),
                to: link.peer.to_string(),
                path: link.path,
                relay: link.relay.map(|relay| relay.to_string()),
                rtt_ms: link.rtt_ms,
                loss: None,
                rate: exact.then_some(link.bytes_per_sec),
                rate_band: (!exact).then(|| rate_band(link.bytes_per_sec)),
                age_secs: report.age_secs,
            }
        }));
    }
    links
}

fn counters(series: &[TrafficSeries]) -> Vec<TrafficTick> {
    series
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
        .collect()
}

async fn daemon_boot(shared: &Shared, daemon: &Daemon) -> Result<Boot, ApiError> {
    let snapshot = read(shared, daemon, true).await?;
    let mut client = daemon.connect().await?;
    let Response::Traffic(traffic) = client.request(Request::Traffic).await? else {
        return Err(ApiError::unexpected());
    };
    let pending = daemon_pending(&mut client, &snapshot.status).await?;
    let mut peers = BTreeMap::new();
    let mut link_ticks = Vec::new();
    for network in &snapshot.networks {
        let list = network.peers.as_deref().unwrap_or_default();
        peers.insert(network.summary.id.to_string(), json::to_value(&list)?);
        link_ticks.extend(links(
            &network.summary,
            &network.reports,
            snapshot.status.node,
        ));
    }
    Ok(Boot {
        boot: shared.boot.clone(),
        seq: 0,
        server_time_ms: now_ms(),
        versions: snapshot.view.versions.clone(),
        last_tick: Some(Tick {
            at_ms: now_ms(),
            traffic: counters(&traffic),
            links: link_ticks,
        }),
        traffic_tail: json::to_value(&tail(traffic))?,
        status: snapshot.view,
        peers,
        pending,
    })
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
            my_settings: None,
            id: NetworkId::from_bytes([9; 32]),
            name: "home".into(),
            peers_online: 0,
            peers_total: 0,
            relays: 0,
            problems: Vec::new(),
            admin: false,
        };
        let reports = [report(own, 5_000), report(other, 2_000_000)];
        let shown: Vec<_> = links(&network, &reports, Some(own))
            .into_iter()
            .map(|link| (link.rate, link.rate_band))
            .collect();
        assert_eq!(shown, [(Some(5_000), None), (None, Some(RateBand::Busy))]);
        network.admin = true;
        assert!(
            links(&network, &reports, Some(own))
                .iter()
                .all(|link| link.rate.is_some() && link.rate_band.is_none())
        );
        assert_eq!(
            [999, 1_000, 1_000_000, 1_000_001].map(rate_band),
            [
                RateBand::Idle,
                RateBand::Light,
                RateBand::Light,
                RateBand::Busy
            ]
        );
    }
}
