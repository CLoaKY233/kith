//! Exits: which one this device uses, and which devices may be one.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::status::{local, recorded_network};
use crate::api::{Answer, ApiError, accepted, as_admin, network_id, node_id, status_of, summary};
use crate::ipc::Source;
use crate::wire::{ApiErrorCode, ExitIn, ExitView, GrantIn};

/// `GET /api/networks/:net/exits`: every device an admin allowed as an exit, and who
/// of this device uses it.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => {
            let fixture = &*sim.read();
            recorded_network(fixture, &net)?;
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
    let Response::Exits(exits) = client.request(Request::ListExits { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let Response::Peers(peers) = client.request(Request::Peers { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let in_use = status
        .exit_node
        .as_deref()
        .and_then(|exit| exit.split_once('/'))
        .filter(|(network_name, _)| *network_name == summary.name)
        .map(|(_, device)| device.to_owned());
    let me = match status.node {
        Some(node) => local(&mut client, summary, node).await?.0.name,
        None => String::new(),
    };
    let views: Vec<ExitView> = exits
        .iter()
        .filter_map(|node| peers.iter().find(|peer| peer.node == *node))
        .map(|peer| ExitView {
            network: net.clone(),
            node: peer.node.to_string(),
            name: peer.name.clone(),
            allowed: true,
            access: Vec::new(),
            in_use_by: if in_use.as_deref() == Some(peer.name.as_str()) && !me.is_empty() {
                vec![me.clone()]
            } else {
                Vec::new()
            },
        })
        .collect();
    Ok(Json(json!(views)))
}

/// `POST /api/exit`: uses an exit, or stops with `node: null`.
pub(crate) async fn use_exit(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<ExitIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.use_exit(&shared.live, &input),
        Source::Daemon(daemon) => daemon,
    };
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
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.grant(&net, &node, input.allowed),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let request = Request::SetExitGrant {
        network,
        node: node_id(&node)?,
        allowed: input.allowed,
    };
    let Response::ExitGrantSet = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
