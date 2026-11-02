//! `GET /api/traffic`: the last hour of traffic samples for every device, as the server
//! keeps them.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::api::Answer;
use crate::ipc::Source;
use crate::{Shared, json};

/// `GET /api/traffic`.
pub(crate) async fn traffic(State(shared): State<Arc<Shared>>) -> Answer {
    match &shared.source {
        Source::Fixture(sim) => Ok(Json(sim.read().traffic.clone())),
        Source::Daemon(_) => Ok(Json(json::to_value(
            &shared.live.traffic(crate::traffic::CAPACITY),
        )?)),
    }
}
