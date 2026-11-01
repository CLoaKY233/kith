//! `/api/networks/:net/invites`: invite links.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::api::{Answer, ApiError, network_id, qr_svg};
use crate::ipc::Source;
use crate::wire::{Invite, InviteIn};
use crate::{Shared, now_ms};

/// `GET /api/networks/:net/invites`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(fixture) => Ok(Json(json!(
            fixture.invites.get(&net).cloned().unwrap_or_default()
        ))),
        Source::Daemon(_) => Err(ApiError::not_implemented("listing invites")),
    }
}

/// `POST /api/networks/:net/invites`.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<InviteIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::CreateInvite {
        network: network_id(&net)?,
        expires_in_secs: input.expires_secs,
        max_uses: input.uses,
    };
    let Response::Invite { invite_link } = daemon.ask(request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(Json(json!(Invite {
        id: String::new(),
        qr_svg: qr_svg(&invite_link),
        link: invite_link,
        expires_at_ms: now_ms().saturating_add(input.expires_secs.saturating_mul(1000)),
        uses: input.uses,
        used_by: Vec::new(),
    })))
}

/// `POST /api/networks/:net/invites/:id/revoke`.
pub(crate) async fn revoke(
    State(shared): State<Arc<Shared>>,
    Path((_net, _id)): Path<(String, String)>,
) -> Answer {
    match &shared.source {
        Source::Fixture(_) => Err(ApiError::fixture()),
        Source::Daemon(_) => Err(ApiError::not_implemented("revoking an invite")),
    }
}
