//! This device's roles: `/api/server` (exit and relay for others) and
//! `/api/wireguard/import` (a plain WireGuard config run as a network).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, status_of};
use crate::ipc::Source;
use crate::sim::Sim;
use crate::wire::{ImportIn, Imported, ServerView};

/// `GET /api/server`: whether this device is an exit and a relay for others.
pub(crate) async fn get(State(shared): State<Arc<Shared>>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.server(),
        Source::Daemon(daemon) => daemon,
    };
    let status = status_of(daemon).await?;
    Ok(Json(json!(ServerView {
        exit: status.exit,
        relay: status.relay,
    })))
}

/// `POST /api/server` (tier 3): offers this device as an exit and/or relay, or stops.
pub(crate) async fn set(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<ServerView>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.set_server(input),
        Source::Daemon(daemon) => daemon,
    };
    shared
        .api
        .confirm("change what this device offers other devices")
        .await?;
    let request = Request::ServerMode {
        exit: input.exit,
        relay: input.relay,
    };
    let Response::ServerModeSet { .. } = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/wireguard/import` (tier 3): runs a `wg-quick` config as a network.
pub(crate) async fn import(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<ImportIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(_) => return Sim::import(&input),
        Source::Daemon(daemon) => daemon,
    };
    shared
        .api
        .confirm(&format!(
            "connect this device to the WireGuard network {}",
            input.name
        ))
        .await?;
    let request = Request::ImportWireGuard {
        name: input.name,
        conf_text: input.conf_text,
    };
    let Response::Imported { network } = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(Json(json!(Imported {
        network: network.to_string(),
    })))
}
