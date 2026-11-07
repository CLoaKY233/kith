//! `/api/history`: what the Console kept beyond the daemon's last hour, for 30 days.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError};
use crate::console::stored;
use crate::store::RETENTION_MS;
use crate::wire::{JournalPage, StoredEntry, TrafficHistory, TrafficMinute};

/// Entries per page unless asked otherwise, and at most.
const PAGE: u32 = 200;
const MAX_PAGE: u32 = 1000;
/// The traffic span unless asked otherwise: a day.
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
pub(crate) struct JournalQuery {
    before: Option<i64>,
    limit: Option<u32>,
    network: Option<String>,
}

/// `GET /api/history/journal?before=&limit=&network=`: newest first.
pub(crate) async fn journal(
    State(shared): State<Arc<Shared>>,
    Query(query): Query<JournalQuery>,
) -> Answer {
    let limit = query.limit.unwrap_or(PAGE).clamp(1, MAX_PAGE);
    let rows = stored(
        shared
            .store
            .journal(query.before, query.network.as_deref(), limit),
    )?;
    let full = rows.len() == usize::try_from(limit).unwrap_or(usize::MAX);
    let next_before = full.then(|| rows.last().map(|row| row.id)).flatten();
    let entries = rows
        .into_iter()
        .map(|row| {
            Ok(StoredEntry {
                id: row.id,
                boot: row.boot,
                entry: serde_json::from_str(&row.entry)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(json!(JournalPage {
        entries,
        next_before,
    })))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TrafficQuery {
    network: Option<String>,
    node: Option<String>,
    since_ms: Option<u64>,
    until_ms: Option<u64>,
}

/// `GET /api/history/traffic?network=&node=&since_ms=&until_ms=`: a day unless asked,
/// 30 days at most.
pub(crate) async fn traffic(
    State(shared): State<Arc<Shared>>,
    Query(query): Query<TrafficQuery>,
) -> Answer {
    let until_ms = query.until_ms.unwrap_or_else(|| shared.clock.now_ms());
    let since_ms = query
        .since_ms
        .unwrap_or_else(|| until_ms.saturating_sub(DAY_MS))
        .max(until_ms.saturating_sub(RETENTION_MS));
    if since_ms >= until_ms {
        return Err(ApiError::bad("since_ms must be before until_ms"));
    }
    let rows = stored(shared.store.traffic(
        query.network.as_deref(),
        query.node.as_deref(),
        since_ms,
        until_ms,
    ))?;
    let mut devices: BTreeMap<(String, String), Vec<TrafficMinute>> = BTreeMap::new();
    for row in rows {
        devices
            .entry((row.network, row.node))
            .or_default()
            .push(TrafficMinute {
                minute_ms: row.minute_ms,
                rx_bytes: row.rx_bytes,
                tx_bytes: row.tx_bytes,
            });
    }
    let history: Vec<TrafficHistory> = devices
        .into_iter()
        .map(|((network, node), minutes)| TrafficHistory {
            network,
            node,
            minutes,
        })
        .collect();
    Ok(Json(json!(history)))
}
