//! `GET /api/events`: the daemon's journal as server-sent events.

use core::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt as _;
use http_body_util::channel::Channel;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Response, StatusCode};
use kith_ipc::Entry;

use crate::api::{ApiError, connect};
use crate::{Body, State, json, session};

/// How often an idle stream gets a comment, so proxies and browsers keep it open.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

fn frame(entry: &Entry) -> Result<Bytes, serde_json::Error> {
    let (kind, data) = json::entry(entry)?;
    Ok(Bytes::from(format!(
        "id: {}\nevent: {kind}\ndata: {data}\n\n",
        entry.seq
    )))
}

/// The first entry wanted: after `Last-Event-ID` when the browser resumes, else `since`.
fn since(headers: &HeaderMap, query: Option<&str>) -> Option<u64> {
    let resumed = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .and_then(|last| last.checked_add(1));
    resumed.or_else(|| session::query_param(query, "since")?.parse().ok())
}

pub(crate) async fn stream(
    state: &State,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<Response<Body>, ApiError> {
    let client = connect(state).await?;
    let (_, mut entries) = client.journal(since(headers, query)).await?;
    let (mut sender, body) = Channel::<Bytes>::new(16);
    tokio::spawn(async move {
        let mut keep_alive =
            tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
        loop {
            let chunk = tokio::select! {
                entry = entries.recv() => match entry {
                    Some(Ok(entry)) => match frame(&entry) {
                        Ok(chunk) => chunk,
                        Err(_) => continue,
                    },
                    Some(Err(_)) | None => break,
                },
                _ = keep_alive.tick() => Bytes::from_static(b": keep-alive\n\n"),
            };
            if sender.send_data(chunk).await.is_err() {
                break;
            }
        }
    });
    let mut response = Response::new(body.boxed());
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
