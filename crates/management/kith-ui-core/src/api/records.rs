//! `/api/networks/:net/records`: the names in a network.

use core::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{DnsRecord, RecordInfo, Request, Response};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, network_id};
use crate::ipc::Source;
use crate::wire::{RecordIn, RecordType, RecordView};

fn view(record: &RecordInfo) -> RecordView {
    let (kind, value) = match &record.record {
        DnsRecord::A(address) => (RecordType::A, address.to_string()),
        DnsRecord::Aaaa(address) => (RecordType::Aaaa, address.to_string()),
        DnsRecord::Cname(target) => (RecordType::Cname, target.clone()),
        DnsRecord::Txt(text) => (RecordType::Txt, text.clone()),
        // DnsRecord is non_exhaustive; a future kind shows as its debug text.
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

/// `GET /api/networks/:net/records`.
pub(crate) async fn list(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    match &shared.source {
        Source::Fixture(fixture) => Ok(Json(json!(
            fixture.records.get(&net).cloned().unwrap_or_default()
        ))),
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
pub(crate) async fn set(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<RecordIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::SetRecord {
        network: network_id(&net)?,
        record: record(input.kind, &input.value)?,
        name: input.name,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}

/// `POST /api/networks/:net/records/:name/delete`.
pub(crate) async fn delete(
    State(shared): State<Arc<Shared>>,
    Path((net, name)): Path<(String, String)>,
) -> Answer {
    let Source::Daemon(daemon) = &shared.source else {
        return Err(ApiError::fixture());
    };
    let request = Request::DeleteRecord {
        network: network_id(&net)?,
        name,
    };
    daemon.ask(request).await?;
    Ok(accepted())
}
