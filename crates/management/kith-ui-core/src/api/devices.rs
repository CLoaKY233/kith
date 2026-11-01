//! `/api/networks/:net/devices/:node/...`: renaming and removing devices, and leaving.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{DnsRecord, EntryKind, PeerInfo, RecordInfo, Request, Response};
use kith_types::{NetworkId, NodeId};

use crate::Shared;
use crate::api::records::{named, relative};
use crate::api::{
    Answer, ApiError, accepted, as_admin, daemon, forget, history, lock, network_id, node_id,
    settled, status_of, summary,
};
use crate::ipc::Daemon;
use crate::wire::{ApiErrorCode, RemoveIn, RenameIn};

/// `POST /api/networks/:net/devices/:node/rename`: the daemon has no rename yet, so this
/// answers 501.
pub(crate) async fn rename(
    State(shared): State<Arc<Shared>>,
    Path((_net, _node)): Path<(String, String)>,
    input: Result<Json<RenameIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    daemon(&shared)?;
    Err(ApiError::not_implemented("renaming a device"))
}

/// Whether `record` points at `device`: by its addresses, or as an alias of its name.
fn points_at(record: &RecordInfo, device: &PeerInfo, own: &[RecordInfo], suffix: &str) -> bool {
    let full = format!("{}.{suffix}", device.name);
    if let DnsRecord::Cname(target) = &record.record {
        let target = target.trim_end_matches('.');
        target == full || target == device.name
    } else {
        matches!(record.record, DnsRecord::A(_) | DnsRecord::Aaaa(_))
            && own.iter().any(|mine| mine.record == record.record)
    }
}

/// The names an admin set that point at `node`, as the daemon stores them.
async fn names_of(
    daemon: &Daemon,
    network: NetworkId,
    node: NodeId,
) -> Result<Vec<String>, ApiError> {
    let Response::Peers(peers) = daemon.ask(Request::Peers { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let Some(device) = peers.iter().find(|peer| peer.node == node) else {
        return Ok(Vec::new());
    };
    let (records, suffix) = named(daemon, network).await?;
    let full = format!("{}.{suffix}", device.name);
    let (own, custom): (Vec<_>, Vec<_>) = records
        .into_iter()
        .partition(|record| record.device && record.name == full);
    let names: BTreeSet<String> = custom
        .iter()
        .filter(|record| !record.device && points_at(record, device, &own, &suffix))
        .map(|record| relative(&record.name, &suffix))
        .collect();
    Ok(names.into_iter().collect())
}

/// When the device left the network, by the journal, for the 409 of a remove that lost
/// the race.
async fn removed_at(daemon: &Daemon, network: NetworkId, node: NodeId) -> Option<u64> {
    history(daemon).await.ok()?.iter().rev().find_map(|entry| {
        let EntryKind::DeviceRemoved {
            network: net,
            node: device,
            ..
        } = &entry.kind
        else {
            return None;
        };
        (*net == network && *device == node).then_some(entry.at_ms)
    })
}

/// Takes this device out of `network` once the OS confirms the person at this computer,
/// and forgets the invites and phone configs this server held for it.
async fn leave_network(shared: &Shared, daemon: &Daemon, network: NetworkId) -> Answer {
    let status = status_of(daemon).await?;
    let name = summary(&status, network)?.name.clone();
    shared
        .api
        .confirm(&format!("take this device out of {name}"))
        .await?;
    let Response::Left = daemon.ask(Request::Leave { network }).await? else {
        return Err(ApiError::unexpected());
    };
    forget(&shared.api, network);
    Ok(accepted())
}

/// `POST /api/networks/:net/devices/:node/remove` (tier 2; tier 3 for this device).
///
/// With `lost`, the names an admin pointed at the device go too. A device another admin
/// removed first answers 409, so the page says so and that nothing was signed. Removing
/// this device is leaving, so it waits for the OS like `leave`.
pub(crate) async fn remove(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<RemoveIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = daemon(&shared)?;
    let (network, node) = (network_id(&net)?, node_id(&node)?);
    if status_of(daemon).await?.node == Some(node) {
        return leave_network(&shared, daemon, network).await;
    }
    let names = if input.lost {
        names_of(daemon, network, node).await?
    } else {
        Vec::new()
    };
    match as_admin(daemon, network, Request::RemoveDevice { network, node }).await {
        Ok(Response::DeviceRemoved) => {}
        Ok(_) => return Err(ApiError::unexpected()),
        Err(error) if error.code() == ApiErrorCode::NotFound => {
            return Err(match removed_at(daemon, network, node).await {
                Some(at_ms) => settled(None, at_ms, None),
                None => error,
            });
        }
        Err(error) => return Err(error),
    }
    lock(&shared.api.phones).remove(&(network.to_string(), node.to_string()));
    for name in names {
        match daemon.ask(Request::DeleteRecord { network, name }).await {
            Ok(_) => {}
            Err(error) if error.code() == ApiErrorCode::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(accepted())
}

/// `POST /api/networks/:net/leave` (tier 3): takes this device out of the network once
/// the OS confirms the person at this computer.
pub(crate) async fn leave(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = daemon(&shared)?;
    leave_network(&shared, daemon, network_id(&net)?).await
}
