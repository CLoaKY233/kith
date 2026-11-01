//! `/api/networks/:net/phones`: devices on plain WireGuard, such as phones.
//!
//! Making the phone is the flow's one ledger write; its config holds the phone's private
//! key, so this server keeps it and shows it only after the OS confirms the person at this
//! computer (tier 3).

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{
    Answer, ApiError, HeldPhone, as_admin, daemon, lock, network_id, qr_svg, summary,
};
use crate::wire::{PhoneConfig, PhoneConfigIn, PhoneCreated, PhoneIn};

/// `POST /api/networks/:net/phones`.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<PhoneIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = daemon(&shared)?;
    let network = network_id(&net)?;
    let request = Request::ExportWireGuard {
        network,
        device_name: input.name,
        tags: input.tags,
    };
    let Response::WireGuardConfig {
        device,
        mesh,
        everything,
        ..
    } = as_admin(daemon, network, request).await?
    else {
        return Err(ApiError::unexpected());
    };
    let Response::Peers(peers) = daemon.ask(Request::Peers { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let phone = peers
        .iter()
        .find(|peer| peer.name == device && peer.gateway.is_some())
        .ok_or_else(ApiError::unexpected)?;
    let created = PhoneCreated {
        name: device,
        node: phone.node.to_string(),
        gateway: phone.gateway.clone().unwrap_or_default(),
        everything_available: everything.is_some(),
    };
    lock(&shared.api.phones).insert((net, created.node.clone()), HeldPhone { mesh, everything });
    Ok(Json(json!(created)))
}

/// `POST /api/networks/:net/phones/:node/config`: the private config, once the OS
/// confirms the person at this computer.
pub(crate) async fn config(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<PhoneConfigIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = daemon(&shared)?;
    let held = lock(&shared.api.phones)
        .get(&(net.clone(), node.clone()))
        .cloned()
        .ok_or_else(|| {
            ApiError::not_found(
                "this phone's config was made before kith-ui started; remove the phone and add it again",
            )
        })?;
    let config = if input.everything {
        held.everything
            .ok_or_else(|| ApiError::bad("this device offers no exit for all traffic"))?
    } else {
        held.mesh
    };
    let status = crate::api::status_of(daemon).await?;
    let network = summary(&status, network_id(&net)?)?.name.clone();
    shared
        .api
        .confirm(&format!("show the private key of a new phone in {network}"))
        .await?;
    Ok(Json(json!(PhoneConfig {
        qr_svg: qr_svg(&config),
        config,
    })))
}
