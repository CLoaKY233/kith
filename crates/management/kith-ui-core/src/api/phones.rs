//! `/api/networks/:net/phones`: devices on plain WireGuard, such as phones.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};

use crate::Shared;
use crate::api::{Answer, ApiError};
use crate::ipc::Source;
use crate::wire::{PhoneConfigIn, PhoneIn};

/// `POST /api/networks/:net/phones`: the one ledger write in adding a phone.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Path(_net): Path<String>,
    input: Result<Json<PhoneIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("adding a phone")),
    }
}

/// `POST /api/networks/:net/phones/:node/config`: reveals the private config after the
/// OS confirms the person at this computer (tier 3).
pub(crate) async fn config(
    State(shared): State<Arc<Shared>>,
    Path((_net, _node)): Path<(String, String)>,
    input: Result<Json<PhoneConfigIn>, JsonRejection>,
) -> Answer {
    let Json(_input) = input?;
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("showing a phone's config")),
    }
}
