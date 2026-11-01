//! Exits: which one this device uses, and which devices may be one.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::status::local;
use crate::api::{
    Answer, ApiError, accepted, as_admin, daemon, network_id, node_id, status_of, summary,
};
use crate::ipc::Source;
use crate::wire::{ApiErrorCode, ExitIn, ExitView, GrantIn};

/// `GET /api/networks/:net/exits`.
///
/// The daemon does not list the devices allowed as exits yet, so only the exit this
/// device uses appears.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(fixture) => {
            return Ok(Json(json!(
                fixture.exits.get(&net).cloned().unwrap_or_default()
            )));
        }
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let mut client = daemon.connect().await?;
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let summary = summary(&status, network)?;
    let Some(exit) = status
        .exit_node
        .as_deref()
        .and_then(|exit| exit.split_once('/'))
        .filter(|(network_name, _)| *network_name == summary.name)
        .map(|(_, device)| device.to_owned())
    else {
        return Ok(Json(json!([])));
    };
    let Response::Peers(peers) = client.request(Request::Peers { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let Some(peer) = peers.iter().find(|peer| peer.name == exit) else {
        return Ok(Json(json!([])));
    };
    let me = match status.node {
        Some(node) => local(&mut client, summary, node).await?.0.name,
        None => String::new(),
    };
    Ok(Json(json!([ExitView {
        network: net,
        node: peer.node.to_string(),
        name: peer.name.clone(),
        allowed: true,
        access: Vec::new(),
        in_use_by: if me.is_empty() { Vec::new() } else { vec![me] },
    }])))
}

/// `POST /api/exit`: uses an exit, or stops with `node: null`.
pub(crate) async fn use_exit(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<ExitIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = daemon(&shared)?;
    let network = match (&input.network, &input.node) {
        (Some(text), _) => network_id(text)?,
        (None, Some(_)) => return Err(ApiError::bad("an exit needs its network")),
        (None, None) => status_of(daemon)
            .await?
            .networks
            .first()
            .map(|summary| summary.id)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::CONFLICT,
                    ApiErrorCode::NotFound,
                    "this device is in no network",
                )
            })?,
    };
    let node = input.node.as_deref().map(node_id).transpose()?;
    let Response::ExitSet = daemon.ask(Request::UseExit { network, node }).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/networks/:net/exits/:node` (tier 2): allows or stops a device being an exit.
pub(crate) async fn grant(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<GrantIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let network = network_id(&net)?;
    let request = Request::SetExitGrant {
        network,
        node: node_id(&node)?,
        allowed: input.allowed,
    };
    let Response::ExitGrantSet = as_admin(daemon(&shared)?, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
