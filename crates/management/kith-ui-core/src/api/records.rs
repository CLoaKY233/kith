//! `/api/networks/:net/records`: the names in a network.

use core::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{DnsRecord, RecordInfo, Request, Response};
use kith_types::NetworkId;
use serde_json::json;

use crate::api::status::{recorded_network, suffix};
use crate::api::{
    Answer, ApiError, accepted, as_admin, daemon, network_id, settled, status_of, summary,
};
use crate::ipc::{Daemon, Source};
use crate::wire::{RecordIn, RecordType, RecordView};
use crate::{Shared, now_ms};

fn view(record: &RecordInfo) -> RecordView {
    let (kind, value) = match &record.record {
        DnsRecord::A(address) => (RecordType::A, address.to_string()),
        DnsRecord::Aaaa(address) => (RecordType::Aaaa, address.to_string()),
        DnsRecord::Cname(target) => (RecordType::Cname, target.clone()),
        DnsRecord::Txt(text) => (RecordType::Txt, text.clone()),
        other => (RecordType::Txt, format!("{other:?}")),
    };
    RecordView {
        name: record.name.clone(),
        kind,
        value,
        device: record.device,
        set_by: None,
    }
}

fn record(kind: RecordType, value: &str) -> Result<DnsRecord, ApiError> {
    let invalid = || ApiError::bad(format!("{value} is not a valid {kind:?} value"));
    match kind {
        RecordType::A => value
            .parse::<Ipv4Addr>()
            .map(DnsRecord::A)
            .map_err(|_| invalid()),
        RecordType::Aaaa => value
            .parse::<Ipv6Addr>()
            .map(DnsRecord::Aaaa)
            .map_err(|_| invalid()),
        RecordType::Cname => Ok(DnsRecord::Cname(value.to_owned())),
        RecordType::Txt => Ok(DnsRecord::Txt(value.to_owned())),
    }
}

/// The network's names, and the suffix they end in, such as `home.kith.internal`.
pub(crate) async fn named(
    daemon: &Daemon,
    network: NetworkId,
) -> Result<(Vec<RecordInfo>, String), ApiError> {
    let status = status_of(daemon).await?;
    let suffix = suffix(&summary(&status, network)?.name);
    let Response::Records(records) = daemon.ask(Request::Records { network }).await? else {
        return Err(ApiError::unexpected());
    };
    Ok((records, suffix))
}

/// A name without the network's suffix, as the daemon stores it: `nas`, not
/// `nas.home.kith.internal`.
pub(crate) fn relative(name: &str, suffix: &str) -> String {
    name.strip_suffix(suffix)
        .and_then(|rest| rest.strip_suffix('.'))
        .unwrap_or(name)
        .to_owned()
}

/// `GET /api/networks/:net/records`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(fixture) => {
            recorded_network(fixture, &net)?;
            Ok(Json(json!(
                fixture.records.get(&net).cloned().unwrap_or_default()
            )))
        }
        Source::Daemon(daemon) => {
            let network = network_id(&net)?;
            let Response::Records(records) = daemon.ask(Request::Records { network }).await? else {
                return Err(ApiError::unexpected());
            };
            Ok(Json(json!(records.iter().map(view).collect::<Vec<_>>())))
        }
    }
}

/// `POST /api/networks/:net/records`.
///
/// With `based_on`, the value the edit started from, the change is refused with 409 when
/// the name now holds something else, and `conflict.op` carries what it holds.
pub(crate) async fn set(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<RecordIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = daemon(&shared)?;
    let network = network_id(&net)?;
    let data = record(input.kind, &input.value)?;
    if let Some(based_on) = &input.based_on {
        let (records, suffix) = named(daemon, network).await?;
        let name = relative(&input.name, &suffix);
        let current: Vec<String> = records
            .iter()
            .filter(|record| !record.device && relative(&record.name, &suffix) == name)
            .map(|record| view(record).value)
            .collect();
        if !current.contains(based_on) {
            return Err(settled(None, now_ms(), current.into_iter().next()));
        }
    }
    let request = Request::SetRecord {
        network,
        name: input.name,
        record: data,
    };
    let Response::RecordSet = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

/// `POST /api/networks/:net/records/:name/delete`. `name` may be the full name the list
/// shows or the short one.
pub(crate) async fn delete(
    State(shared): State<Arc<Shared>>,
    Path((net, name)): Path<(String, String)>,
) -> Answer {
    let daemon = daemon(&shared)?;
    let network = network_id(&net)?;
    let status = status_of(daemon).await?;
    let suffix = suffix(&summary(&status, network)?.name);
    let request = Request::DeleteRecord {
        network,
        name: relative(&name, &suffix),
    };
    let Response::RecordDeleted = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
