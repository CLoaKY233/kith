//! Joining: starting a network, joining one, asking to join, and admins answering requests.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::Request;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, network_id, node_id};
use crate::ipc::Source;
use crate::wire::{CreateNetworkIn, JoinIn, RequestJoinIn};

/// `POST /api/networks`.
pub(crate) async fn create_network(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<CreateNetworkIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    daemon
        .ask(Request::CreateNetwork {
            name: input.name,
            device_name: input.device_name,
        })
        .await?;
    Ok(accepted())
}

/// `POST /api/join`.
pub(crate) async fn join(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<JoinIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    daemon
        .ask(Request::Join {
            invite_link: input.invite_link,
            device_name: input.device_name,
        })
        .await?;
    Ok(accepted())
}

/// `POST /api/join/request`.
pub(crate) async fn request(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<RequestJoinIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("asking to join")),
    }
}

/// `POST /api/networks/:net/joins/:node/approve`.
pub(crate) async fn approve(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
) -> Answer {
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::ApproveJoin {
        network: network_id(&net)?,
        node: node_id(&node)?,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}

/// `POST /api/networks/:net/joins/:node/deny`.
pub(crate) async fn deny(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
) -> Answer {
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::DenyJoin {
        network: network_id(&net)?,
        node: node_id(&node)?,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}
