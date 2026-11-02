//! Joining: starting a network, joining one, asking to join, and admins answering requests.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{EntryKind, Request, Response};
use kith_types::{NetworkId, NodeId};
use serde_json::json;

use crate::api::codes::check_code;
use crate::api::status::{JOIN_LIFETIME_MS, recorded_network};
use crate::api::{
    Answer, ApiError, accepted, as_admin, history, network_id, node_id, settled, status_of,
};
use crate::ipc::{Daemon, Source};
use crate::wire::{
    ApiErrorCode, ApproveIn, CreateNetworkIn, JoinCode, JoinIn, JoinRequestState, RequestJoinIn,
    Resolution,
};
use crate::{Shared, now_ms};

/// `POST /api/networks`.
pub(crate) async fn create_network(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<CreateNetworkIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.create_network(&input),
        Source::Daemon(daemon) => daemon,
    };
    let request = Request::CreateNetwork {
        name: input.name,
        device_name: input.device_name,
    };
    let Response::NetworkCreated { .. } = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/join`.
pub(crate) async fn join(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<JoinIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.join(&input),
        Source::Daemon(daemon) => daemon,
    };
    let request = Request::Join {
        invite_link: input.invite_link,
        device_name: input.device_name,
    };
    let Response::Joined { .. } = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/join/request`: asks an admin to let this device in, or learns the answer.
///
/// The page sends it again every few seconds while it waits; the daemon reuses the same
/// signed request, so admins see one request and one check code.
pub(crate) async fn request(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<RequestJoinIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.request_join(&input.request_code),
        Source::Daemon(daemon) => daemon,
    };
    let mut client = daemon.connect().await?;
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let code = status.node.as_ref().map(check_code).unwrap_or_default();
    let answer = client
        .request(Request::RequestJoin {
            request_code: input.request_code,
            device_name: input.device_name,
        })
        .await?;
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any other answer is a protocol error"
    )]
    let state = match answer {
        Response::JoinPending {
            network,
            network_name,
        } => JoinRequestState {
            network: Some(network.to_string()),
            network_name,
            check_code: code,
            resolution: None,
        },
        Response::JoinDenied { network } => JoinRequestState {
            network: Some(network.to_string()),
            network_name: String::new(),
            check_code: code,
            resolution: Some(Resolution::Denied),
        },
        Response::Joined { network, .. } => {
            let network_name = status_of(daemon)
                .await?
                .networks
                .into_iter()
                .find(|summary| summary.id == network)
                .map(|summary| summary.name)
                .unwrap_or_default();
            JoinRequestState {
                network: Some(network.to_string()),
                network_name,
                check_code: code,
                resolution: Some(Resolution::Approved),
            }
        }
        _ => return Err(ApiError::unexpected()),
    };
    Ok(Json(json!(state)))
}

/// `GET /api/networks/:net/request-code`: the code a new device asks to join with.
pub(crate) async fn request_code(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
) -> Answer {
    let code = match &shared.source {
        Source::Fixture(sim) => {
            let fixture = &*sim.read();
            let name = recorded_network(fixture, &net)?
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default();
            format!("kith://request/{name}-recorded")
        }
        Source::Daemon(daemon) => {
            let network = network_id(&net)?;
            let Response::RequestCode { code } =
                daemon.ask(Request::RequestCode { network }).await?
            else {
                return Err(ApiError::unexpected());
            };
            code
        }
    };
    Ok(Json(json!(JoinCode {
        command: format!("kith request {code}"),
        code,
    })))
}

/// Why a request is no longer pending: someone approved it, denied it, or it lapsed.
async fn settled_join(daemon: &Daemon, network: NetworkId, node: NodeId) -> ApiError {
    let member = match daemon.ask(Request::Peers { network }).await {
        Ok(Response::Peers(peers)) => peers.iter().any(|peer| peer.node == node),
        Ok(_) => false,
        Err(error) => return error,
    };
    let entries = history(daemon).await.unwrap_or_default();
    let now = now_ms();
    let at = |wanted: fn(&EntryKind) -> Option<(NetworkId, NodeId)>| {
        entries
            .iter()
            .rev()
            .find(|entry| wanted(&entry.kind) == Some((network, node)))
            .map(|entry| entry.at_ms)
    };
    let joined = at(|kind| {
        if let EntryKind::DeviceJoined { network, node, .. } = kind {
            Some((*network, *node))
        } else {
            None
        }
    });
    if member || joined.is_some() {
        return settled(Some(Resolution::Approved), joined.unwrap_or(now), None);
    }
    let asked = at(|kind| {
        if let EntryKind::JoinRequested { network, node, .. } = kind {
            Some((*network, *node))
        } else {
            None
        }
    });
    match asked.map(|at_ms| at_ms.saturating_add(JOIN_LIFETIME_MS)) {
        Some(lapsed) if lapsed <= now => settled(Some(Resolution::Expired), lapsed, None),
        Some(_) | None => settled(Some(Resolution::Denied), now, None),
    }
}

/// What an admin decided about a join request.
#[derive(Debug)]
enum Decision {
    /// Let it in, under this name or the one it asked for.
    Approve(Option<String>),
    /// Turn it down.
    Deny,
}

/// Approves, under `name` when given, or denies; when the request is already settled,
/// 409 says how.
async fn answer(shared: &Shared, net: &str, node: &str, decision: Decision) -> Answer {
    let daemon = match (&shared.source, &decision) {
        (Source::Fixture(sim), Decision::Approve(name)) => {
            return sim.approve(&shared.live, net, node, name.as_deref());
        }
        (Source::Fixture(sim), Decision::Deny) => return sim.deny(net, node),
        (Source::Daemon(daemon), _) => daemon,
    };
    let (network, node) = (network_id(net)?, node_id(node)?);
    let request = if matches!(decision, Decision::Approve(_)) {
        Request::ApproveJoin { network, node }
    } else {
        Request::DenyJoin { network, node }
    };
    match as_admin(daemon, network, request).await {
        Ok(Response::JoinApproved { device }) => {
            if let Decision::Approve(Some(name)) = decision
                && !name.trim().is_empty()
                && name.trim() != device
            {
                let rename = Request::RenameDevice {
                    network,
                    node,
                    name,
                };
                let Response::DeviceRenamed = as_admin(daemon, network, rename).await? else {
                    return Err(ApiError::unexpected());
                };
            }
            Ok(accepted())
        }
        Ok(Response::JoinRequestDenied) => Ok(accepted()),
        Ok(_) => Err(ApiError::unexpected()),
        Err(error) if error.code() == ApiErrorCode::NotFound => {
            Err(settled_join(daemon, network, node).await)
        }
        Err(error) => Err(error),
    }
}

/// `POST /api/networks/:net/joins/:node/approve`: the device joins as a member, under
/// `name` when the admin gave one, else under the name it asked for.
pub(crate) async fn approve(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<ApproveIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    answer(&shared, &net, &node, Decision::Approve(input.name)).await
}

/// `POST /api/networks/:net/joins/:node/deny`.
pub(crate) async fn deny(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
) -> Answer {
    answer(&shared, &net, &node, Decision::Deny).await
}
