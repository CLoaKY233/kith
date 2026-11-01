//! `GET /api/traffic`: the last hour of traffic samples for every device.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use kith_ipc::{Request, Response};

use crate::api::{Answer, ApiError};
use crate::ipc::Source;
use crate::{Shared, json};

/// `GET /api/traffic`.
pub(crate) async fn traffic(State(shared): State<Arc<Shared>>) -> Answer {
    match &shared.source {
        Source::Fixture(fixture) => Ok(Json(fixture.traffic.clone())),
        Source::Daemon(daemon) => {
            let Response::Traffic(series) = daemon.ask(Request::Traffic).await? else {
                return Err(ApiError::unexpected());
            };
            Ok(Json(json::to_value(&series)?))
        }
    }
}
