//! `/api/networks/:net/phones`: devices on plain WireGuard, such as phones.
//!
//! Making the phone is the flow's one ledger write; its config holds the phone's private
//! key, so this server keeps it and shows it only to the signed-in person who made it, or,
//! on the local page, after the OS confirms the person at this computer (tier 3).

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use serde_json::json;

use crate::api::{
    Answer, ApiError, HeldPhone, as_admin, lock, network_id, node_id, qr_svg, summary,
};
use crate::ipc::Source;
use crate::wire::{PhoneConfig, PhoneConfigIn, PhoneCreated, PhoneIn};
use crate::{Shared, now_ms};

/// The most phone configs held at once; past it the oldest is dropped, and that phone
/// has to be made again to show its config.
const MAX_HELD: usize = 64;

/// `POST /api/networks/:net/phones`.
pub(crate) async fn create(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<PhoneIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => {
            let (created, held) = sim.add_phone(&shared.live, &net, &input)?;
            hold(&shared, &net, &created.node, held);
            assign(&shared, &net, &created.node).await;
            return Ok(Json(json!(created)));
        }
        Source::Daemon(daemon) => daemon,
    };
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
    let held = HeldPhone {
        mesh,
        everything,
        made_ms: now_ms(),
        made_by: None,
    };
    hold(&shared, &network.to_string(), &created.node, held);
    assign(&shared, &net, &created.node).await;
    Ok(Json(json!(created)))
}

/// Gives a new phone to the signed-in person who made it. Best effort: a daemon without
/// accounts, or the local link's session, leaves it unassigned.
async fn assign(shared: &Shared, net: &str, node: &str) {
    let Some(account) = crate::accounts::actor() else {
        return;
    };
    if let (Ok(network), Ok(node)) = (network_id(net), node_id(node)) {
        let _assigned = shared.accounts.assign_device(network, node, account).await;
    }
}

/// Keeps a phone's config until the page reveals it, dropping the oldest past [`MAX_HELD`].
fn hold(shared: &Shared, net: &str, node: &str, mut held: HeldPhone) {
    held.made_by = crate::accounts::actor();
    let mut phones = lock(&shared.api.phones);
    phones.insert((net.to_owned(), node.to_owned()), held);
    while phones.len() > MAX_HELD {
        let Some(oldest) = phones
            .iter()
            .min_by_key(|(_, held)| held.made_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        phones.remove(&oldest);
    }
}

/// `POST /api/networks/:net/phones/:node/config`: the private config, once the OS
/// confirms the person at this computer.
pub(crate) async fn config(
    State(shared): State<Arc<Shared>>,
    Path((net, node)): Path<(String, String)>,
    input: Result<Json<PhoneConfigIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let network = network_id(&net)?;
    let key = (network.to_string(), node_id(&node)?.to_string());
    let held = lock(&shared.api.phones)
        .get(&key)
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
    // On the Console a server has no one at its screen to ask: the person who just made
    // the phone, signed in with their passkey, sees its config; nobody else does.
    if let Some(me) = crate::accounts::actor() {
        if held.made_by.as_deref() != Some(me.as_str()) {
            return Err(crate::console::denied(
                "only the person who added this phone can show its config; remove the \
                 phone and add it again to get a new one",
            ));
        }
    } else if let Source::Daemon(daemon) = &shared.source {
        let status = crate::api::status_of(daemon).await?;
        let name = summary(&status, network)?.name.clone();
        shared
            .api
            .confirm(&format!("show the private key of a new phone in {name}"))
            .await?;
    }
    Ok(Json(json!(PhoneConfig {
        qr_svg: qr_svg(&config),
        config,
    })))
}
