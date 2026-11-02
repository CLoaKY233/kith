//! `/api/networks/:net/relays`: relay servers advertised to a network's members.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, as_admin, network_id, node_id};
use crate::ipc::Source;
use crate::wire::{RelayIn, RelayView};

/// `GET /api/networks/:net/relays`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.relays(&net),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let Response::Relays(relays) = daemon.ask(Request::ListRelays { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let relays: Vec<RelayView> = relays
        .into_iter()
        .map(|relay| RelayView {
            name: relay.name,
            address: relay.address,
            node: relay.node.to_string(),
        })
        .collect();
    Ok(Json(json!(relays)))
}

/// `POST /api/networks/:net/relays` (tier 2): advertises a relay, replacing one of the
/// same name.
pub(crate) async fn add(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<RelayIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.add_relay(&net, &input),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let request = Request::AdvertiseRelay {
        network,
        name: input.name,
        address: input.address,
        node: input.node.as_deref().map(node_id).transpose()?,
    };
    let Response::RelayAdvertised { .. } = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/networks/:net/relays/:name/remove` (tier 2).
pub(crate) async fn remove(
    State(shared): State<Arc<Shared>>,
    Path((net, name)): Path<(String, String)>,
) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.remove_relay(&net, &name),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let request = Request::RemoveRelay { network, name };
    let Response::RelayRemoved = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
