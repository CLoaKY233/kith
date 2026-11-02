//! `GET /api/networks/:net/peers`: every other device in a network.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};

use crate::api::status::recorded_network;
use crate::api::{Answer, ApiError, network_id};
use crate::ipc::Source;
use crate::{Shared, json};

/// `GET /api/networks/:net/peers`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(sim) => {
            let fixture = &*sim.read();
            recorded_network(fixture, &net)?;
            Ok(Json(
                fixture
                    .peers
                    .get(&net)
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            ))
        }
        Source::Daemon(daemon) => {
            let network = network_id(&net)?;
            let Response::Peers(peers) = daemon.ask(Request::Peers { network }).await? else {
                return Err(ApiError::unexpected());
            };
            Ok(Json(json::to_value(&peers)?))
        }
    }
}
