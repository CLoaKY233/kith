//! `/api/networks/:net/invites`: invite links.
//!
//! The daemon cannot list or revoke invites yet, so the list holds the invites made
//! through this page since `kith-ui` started, until they lapse.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::api::status::recorded_network;
use crate::api::{Answer, ApiError, as_admin, daemon, lock, network_id, qr_svg};
use crate::ipc::Source;
use crate::session::Token;
use crate::wire::{Invite, InviteIn};
use crate::{Shared, now_ms};

/// Hex digits in an invite's id.
const ID_LEN: usize = 8;
/// The longest an invite may last: a year.
const MAX_LIFETIME_SECS: u64 = 365 * 24 * 60 * 60;
/// The most open invites listed per network; past it the oldest leaves the list, though
/// it stays valid until it lapses.
const MAX_LISTED: usize = 256;

/// `GET /api/networks/:net/invites`: open invites, newest first.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let invites = match &shared.source {
        Source::Fixture(fixture) => {
            recorded_network(fixture, &net)?;
            fixture.invites.get(&net).cloned().unwrap_or_default()
        }
        Source::Daemon(_) => {
            let key = network_id(&net)?.to_string();
            let now = now_ms();
            let mut made = lock(&shared.api.invites);
            let Some(open) = made.get_mut(&key) else {
                return Ok(Json(json!([])));
            };
            open.retain(|invite| invite.expires_at_ms > now);
            let newest_first: Vec<Invite> = open.iter().rev().cloned().collect();
            if open.is_empty() {
                made.remove(&key);
            }
            newest_first
        }
    };
    Ok(Json(json!(invites)))
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
    let request = Request::CreateInvite {
        network,
        expires_in_secs: input.expires_secs,
        max_uses: input.uses,
    };
    let Response::Invite { invite_link } = as_admin(daemon(&shared)?, network, request).await?
    else {
        return Err(ApiError::unexpected());
    };
    let id = Token::random()
        .map_err(|_| ApiError::unexpected())?
        .hex()
        .get(..ID_LEN)
        .map(|hex| format!("inv-{hex}"))
        .unwrap_or_default();
    let invite = Invite {
        id,
        qr_svg: qr_svg(&invite_link),
        link: invite_link,
        expires_at_ms: now_ms().saturating_add(input.expires_secs.saturating_mul(1000)),
        uses: input.uses,
        used_by: Vec::new(),
    };
    let now = now_ms();
    let mut made = lock(&shared.api.invites);
    let open = made.entry(network.to_string()).or_default();
    open.retain(|invite| invite.expires_at_ms > now);
    open.push(invite.clone());
    let excess = open.len().saturating_sub(MAX_LISTED);
    open.drain(..excess);
    drop(made);
    Ok(Json(json!(invite)))
}

/// `POST /api/networks/:net/invites/:id/revoke`: the daemon cannot revoke an invite yet,
/// so this answers 501 and the invite stays valid until it lapses.
pub(crate) async fn revoke(
    State(shared): State<Arc<Shared>>,
    Path((_net, _id)): Path<(String, String)>,
) -> Answer {
    daemon(&shared)?;
    Err(ApiError::not_implemented("revoking an invite"))
}
