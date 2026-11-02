//! This device's roles: `/api/server` (exit and relay for others) and
//! `/api/wireguard/import` (a plain WireGuard config run as a network).

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, network_id, status_of};
use crate::ipc::Source;
use crate::wire::{ImportIn, ImportUseIn, Imported, ServerView};

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
        Source::Fixture(sim) => return sim.import(&shared.live, &input),
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

/// `POST /api/wireguard/:net/use`: routes through an imported network, or stops. One
/// that carries all internet traffic replaces the exit in use.
pub(crate) async fn use_import(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<ImportUseIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.use_import(&shared.live, &net, input.active),
        Source::Daemon(daemon) => daemon,
    };
    let request = Request::UseImport {
        network: network_id(&net)?,
        active: input.active,
    };
    let Response::ImportUsed = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/wireguard/:net/remove` (tier 3): removes an imported network and its config.
pub(crate) async fn remove_import(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.remove_import(&net),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let name = status_of(daemon)
        .await?
        .imported
        .into_iter()
        .find(|import| import.network == network)
        .map(|import| import.name)
        .ok_or_else(|| ApiError::not_found("no such imported network"))?;
    shared
        .api
        .confirm(&format!("remove the WireGuard network {name}"))
        .await?;
    let Response::ImportRemoved = daemon.ask(Request::RemoveImport { network }).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
