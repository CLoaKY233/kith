//! Device codes: a person makes a six-character code in the Console, and a computer runs
//! `kith join https://<domain>/j/<code>`, which fetches a single-use invite from the
//! public `GET /j/<code>` and joins with it. The device is then the person's.

use core::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Extension, Path, Request, State};
use axum::http::StatusCode;
use kith_identity::InviteToken;
use kith_types::NetworkId;
use serde_json::json;

use crate::Shared;
use crate::accounts::acting_as;
use crate::api::{Answer, ApiError, qr_svg};
use crate::console::{
    CODE_MS, console_network, device_code, link_origin, link_refusal, normalize_code,
    one_device_invite, require, signed_in, stored,
};
use crate::public::ClientAddr;
use crate::session::Caller;
use crate::store::DeviceCodeRow;
use crate::wire::{ApiErrorCode, DeviceCodeIn, DeviceCodeView, JoinLink, PersonRole};

/// Tries at a fresh code when one collides with a code still kept.
const ATTEMPTS: usize = 4;

/// `POST /api/device-codes`.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    input: Result<Json<DeviceCodeIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let (network, me) = signed_in(&shared, principal.as_ref()).await?;
    let person = match input.person {
        Some(person) if person != me.id => {
            require(&me, PersonRole::Admin)?;
            let known = shared
                .accounts
                .accounts(network)
                .await?
                .iter()
                .any(|account| account.id == person);
            if !known {
                return Err(ApiError::not_found("no such person"));
            }
            person
        }
        Some(_) | None => me.id.clone(),
    };
    let now = shared.clock.now_ms();
    let mut made = None;
    for _ in 0..ATTEMPTS {
        let row = DeviceCodeRow {
            code: device_code()?,
            network: network.to_string(),
            account: person.clone(),
            created_by: me.id.clone(),
            created_ms: now,
            expires_ms: now.saturating_add(CODE_MS),
            used_ms: None,
            invite: None,
            node: None,
        };
        if shared.store.put_device_code(&row).is_ok() {
            made = Some(row);
            break;
        }
    }
    let row = made.ok_or_else(ApiError::unexpected)?;
    let link = format!("{}/j/{}", link_origin(&shared), row.code);
    Ok(Json(json!(DeviceCodeView {
        code: row.code,
        expires_at_ms: row.expires_ms,
        command: format!("kith join {link}"),
        qr_svg: qr_svg(&link),
        link,
        person,
    })))
}

/// `GET /j/{code}`: public, rate-limited per address, and good once.
pub(crate) async fn redeem(
    State(shared): State<Arc<Shared>>,
    Path(code): Path<String>,
    request: Request,
) -> Answer {
    let address = request
        .extensions()
        .get::<ConnectInfo<ClientAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |info| info.0.0.ip());
    let now = shared.clock.now_ms();
    if !shared.limiter.allow(address, now) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::RateLimited,
            "too many tries; wait a minute",
        ));
    }
    let code = normalize_code(&code);
    if code.len() != 6 {
        return Err(ApiError::not_found("no such link or code"));
    }
    // Check, mint, then burn: a code is used up only once its invite exists, so a daemon
    // that is down or refuses leaves the code good for another try.
    let row = link_refusal(stored(shared.store.peek_device_code(&code, now))?)?;
    let network: NetworkId = row.network.parse().map_err(|_| ApiError::unexpected())?;
    let invite_link = acting_as(
        Some(row.created_by.clone()),
        one_device_invite(&shared, network),
    )
    .await
    .map_err(|error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ApiErrorCode::Internal,
            format!(
                "the Console could not make an invite for this code ({}); the code still \
                 works, so try again in a minute, or ask an owner to check that kith is \
                 running on the server",
                error.message()
            ),
        )
    })?;
    let token = InviteToken::parse(&invite_link).map_err(|_| ApiError::unexpected())?;
    let invite = crate::sim::hex(&token.invite_public_key().to_bytes());
    link_refusal(stored(
        shared.store.redeem_device_code(&code, now, &invite),
    )?)?;
    let network_name = console_network(&shared)
        .await?
        .filter(|(id, _, _)| *id == network)
        .map_or_else(|| token.network_name().to_owned(), |(_, name, _)| name);
    Ok(Json(json!(JoinLink {
        invite_link,
        network_name,
    })))
}
