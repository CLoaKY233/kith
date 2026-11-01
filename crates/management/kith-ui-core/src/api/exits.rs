//! Exits: which one this device uses, and which devices may be one.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, network_id, node_id};
use crate::ipc::Source;
use crate::wire::{ApiErrorCode, ExitIn, GrantIn};

/// `GET /api/networks/:net/exits`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(fixture) => Ok(Json(json!(
            fixture.exits.get(&net).cloned().unwrap_or_default()
        ))),
        Source::Daemon(_) => Err(ApiError::not_implemented("listing exits")),
    }
}

/// `POST /api/exit`: uses an exit, or stops with `node: null`.
pub(crate) async fn use_exit(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<ExitIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let network = match (&input.network, &input.node) {
        (Some(text), _) => network_id(text)?,
        (None, Some(_)) => return Err(ApiError::bad("an exit needs its network")),
        (None, None) => {
            let Response::Status(status) = daemon.ask(Request::Status).await? else {
                return Err(ApiError::unexpected());
            };
            status
                .networks
                .first()
                .map(|summary| summary.id)
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        ApiErrorCode::NotFound,
                        "this device is in no network",
                    )
                })?
        }
    };
    let node = input.node.as_deref().map(node_id).transpose()?;
    daemon.ask(Request::UseExit { network, node }).await?;
    Ok(accepted())
}

/// `POST /api/networks/:net/exits/:node`: allows or stops a device being an exit.
pub(crate) async fn grant(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<GrantIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::SetExitGrant {
        network: network_id(&net)?,
        node: node_id(&node)?,
        allowed: input.allowed,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}
