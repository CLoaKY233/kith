//! `/api/networks/:net/invites`: invite links, as the network's ledger holds them.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_identity::InviteToken;
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, as_admin, network_id, qr_svg};
use crate::ipc::Source;
use crate::wire::{Invite, InviteIn, OpenInvite};

/// The longest an invite may last: a year.
const MAX_LIFETIME_SECS: u64 = 365 * 24 * 60 * 60;

/// `GET /api/networks/:net/invites`: open invites, newest first.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.invites(&net),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let Response::Invites(invites) = daemon.ask(Request::ListInvites { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let mut open: Vec<OpenInvite> = invites
        .into_iter()
        .map(|invite| OpenInvite {
            id: invite.id,
            expires_at_ms: invite.expires_at_ms,
            uses_left: invite.uses_left,
            created_at_ms: invite.created_at_ms,
        })
        .collect();
    open.sort_by_key(|invite| core::cmp::Reverse(invite.created_at_ms));
    Ok(Json(json!(open)))
}

/// `POST /api/networks/:net/invites`.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<InviteIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let network = network_id(&net)?;
    if !(1..=MAX_LIFETIME_SECS).contains(&input.expires_secs) {
        return Err(ApiError::bad(format!(
            "an invite lasts 1 to {MAX_LIFETIME_SECS} seconds"
        )));
    }
    if input
        .uses
        .is_some_and(|uses| uses == 0 || uses > u32::from(u16::MAX))
    {
        return Err(ApiError::bad(format!(
            "an invite is used 1 to {} times",
            u16::MAX
        )));
    }
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.create_invite(&net, &input),
        Source::Daemon(daemon) => daemon,
    };
    let request = Request::CreateInvite {
        network,
        expires_in_secs: input.expires_secs,
        max_uses: input.uses,
    };
    let Response::Invite { invite_link } = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    let token = InviteToken::parse(&invite_link).map_err(|_| ApiError::unexpected())?;
    let id = crate::sim::hex(&token.invite_public_key().to_bytes());
    Ok(Json(json!(Invite {
        id,
        qr_svg: qr_svg(&invite_link),
        expires_at_ms: token.expires().as_millis(),
        link: invite_link,
        uses: input.uses,
        used_by: Vec::new(),
    })))
}

/// `POST /api/networks/:net/invites/:id/revoke`: no device can join with it after this.
pub(crate) async fn revoke(
    State(shared): State<Arc<Shared>>,
    Path((net, id)): Path<(String, String)>,
) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.revoke_invite(&net, &id),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let request = Request::RevokeInvite {
        network,
        invite: id,
    };
    let Response::InviteRevoked = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
