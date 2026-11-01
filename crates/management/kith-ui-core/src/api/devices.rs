//! `/api/networks/:net/devices/:node/...`: renaming and removing devices, and leaving.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::Request;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, network_id, node_id};
use crate::ipc::Source;
use crate::wire::{RemoveIn, RenameIn};

/// `POST /api/networks/:net/devices/:node/rename`.
pub(crate) async fn rename(
    State(shared): State<Arc<Shared>>,
    Path((_net, _node)): Path<(String, String)>,
    input: Result<Json<RenameIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("renaming a device")),
    }
}

/// `POST /api/networks/:net/devices/:node/remove`.
pub(crate) async fn remove(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<RemoveIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::RemoveDevice {
        network: network_id(&net)?,
        node: node_id(&node)?,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}

/// `POST /api/networks/:net/leave`: tier 3, so it will wait for the OS to confirm.
pub(crate) async fn leave(State(shared): State<Arc<Shared>>, Path(_net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("leaving a network")),
    }
}
