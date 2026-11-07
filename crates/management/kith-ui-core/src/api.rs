//! `/api/...`: reads and changes, each one or a few IPC requests to the daemon.
//!
//! Every handler reads from [`crate::ipc::Source`]: the daemon, or under `--fixture` the
//! simulator in [`crate::sim`], which changes its copy of the recording.

pub(crate) mod auth;
pub(crate) mod blocked;
pub(crate) mod codes;
pub(crate) mod devicecodes;
pub(crate) mod devices;
pub(crate) mod exits;
pub(crate) mod history;
pub(crate) mod invites;
pub(crate) mod joins;
pub(crate) mod peers;
pub(crate) mod people;
pub(crate) mod phones;
pub(crate) mod policy;
pub(crate) mod records;
pub(crate) mod relays;
pub(crate) mod server;
pub(crate) mod status;
pub(crate) mod traffic;
pub(crate) mod validate;

#[cfg(test)]
mod console_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kith_ipc::{Entry, ErrorCode, IpcError, NetworkSummary, Request, Response as Reply, Status};
use kith_types::{NetworkId, NodeId};
use qrcode::QrCode;
use qrcode::render::svg;
use serde_json::{Value, json};

use crate::ipc::Daemon;
use crate::osauth::{Gate, OsAuthError};
use crate::wire::{ApiErrorBody, ApiErrorCode, Conflict, Host, Outcome, OutcomeState, Resolution};

/// How long reading the daemon's journal history may take before giving up on it.
const HISTORY_TIME: core::time::Duration = core::time::Duration::from_secs(5);

/// A failed request, answered with an [`ApiErrorBody`].
#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code,
                message: message.into(),
                conflict: None,
            },
        }
    }

    /// What went wrong, as the page shows it.
    pub(crate) fn message(&self) -> &str {
        &self.body.message
    }

    pub(crate) fn bad(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidArgument,
            message,
        )
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ApiErrorCode::NotFound, message)
    }

    pub(crate) fn unexpected() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            "the daemon gave an unexpected answer",
        )
    }

    /// Someone else settled it first: a 409 that tells the page who and how, so it shows
    /// their result instead of an error.
    pub(crate) fn conflict(conflict: Conflict) -> Self {
        let mut error = Self::new(
            StatusCode::CONFLICT,
            ApiErrorCode::Conflict,
            "changed by another device first",
        );
        error.body.conflict = Some(conflict);
        error
    }

    pub(crate) const fn code(&self) -> ApiErrorCode {
        self.body.code
    }
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.status, self.body.message)
    }
}

impl core::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        response
    }
}

impl From<IpcError> for ApiError {
    fn from(error: IpcError) -> Self {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "every other failure is the daemon's or the connection's"
        )]
        match error {
            IpcError::Remote { code, message } => {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "other codes are internal failures"
                )]
                let (status, code) = match code {
                    ErrorCode::PermissionDenied => {
                        (StatusCode::FORBIDDEN, ApiErrorCode::PermissionDenied)
                    }
                    ErrorCode::NotFound => (StatusCode::NOT_FOUND, ApiErrorCode::NotFound),
                    ErrorCode::InvalidArgument => {
                        (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidArgument)
                    }
                    ErrorCode::VersionMismatch => {
                        (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::VersionSkew)
                    }
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::Internal),
                };
                Self::new(status, code, message)
            }
            other => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::DaemonDown,
                other.to_string(),
            ),
        }
    }
}

impl From<OsAuthError> for ApiError {
    fn from(error: OsAuthError) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::OsAuthDenied,
            error.to_string(),
        )
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            error.to_string(),
        )
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::bad(rejection.body_text())
    }
}

/// A JSON answer.
pub(crate) type Answer = Result<Json<Value>, ApiError>;

/// The answer to a change the daemon applied. `op` stays `null`: the daemon does not name
/// the ledger op a change signed, and the page shows "Saved on this device".
pub(crate) fn accepted() -> Json<Value> {
    Json(json!(Outcome {
        state: OutcomeState::Accepted,
        op: None,
    }))
}

/// A phone config held until the person reveals it: the daemon hands it over once.
#[derive(Debug, Clone)]
pub(crate) struct HeldPhone {
    /// The config that sends only the network's traffic through kith.
    pub(crate) mesh: String,
    /// The config that sends everything through this device's exit, when it has one.
    pub(crate) everything: Option<String>,
    /// When it was made, so the oldest goes first once too many are held.
    pub(crate) made_ms: u64,
}

/// What the API keeps between requests.
#[derive(Debug)]
pub(crate) struct State {
    /// Who confirms tier-3 changes.
    gate: Gate,
    /// One OS prompt at a time: a second tier-3 request waits for the first.
    prompt: tokio::sync::Mutex<()>,
    /// The computer, read once.
    pub(crate) host: Host,
    /// Phone configs made through this page, per network id and device id, each as the
    /// ids print.
    pub(crate) phones: Mutex<BTreeMap<(String, String), HeldPhone>>,
    /// Recovery words made during setup, per owner, until the setup page shows them once.
    pub(crate) recovery: Mutex<BTreeMap<String, Vec<String>>>,
}

impl State {
    pub(crate) fn new(gate: Gate) -> Self {
        Self {
            gate,
            prompt: tokio::sync::Mutex::new(()),
            host: status::host(),
            phones: Mutex::new(BTreeMap::new()),
            recovery: Mutex::new(BTreeMap::new()),
        }
    }

    /// Asks the person at this computer to confirm a tier-3 change before it is signed.
    pub(crate) async fn confirm(&self, reason: &str) -> Result<(), ApiError> {
        let _one_at_a_time = self.prompt.lock().await;
        Ok(self.gate.confirm(reason).await?)
    }
}

/// Forgets the private configs of the phones of a network this device left.
pub(crate) fn forget(state: &State, network: NetworkId) {
    let key = network.to_string();
    lock(&state.phones).retain(|(net, _), _| *net != key);
}

/// Locks `mutex`; a panic elsewhere while holding it leaves data that is still usable.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn network_id(text: &str) -> Result<NetworkId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a network id"))
}

pub(crate) fn node_id(text: &str) -> Result<NodeId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a device id"))
}

/// The daemon's status.
pub(crate) async fn status_of(daemon: &Daemon) -> Result<Status, ApiError> {
    let Reply::Status(status) = daemon.ask(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(status)
}

/// The network in `status`, or 404.
pub(crate) fn summary(status: &Status, network: NetworkId) -> Result<&NetworkSummary, ApiError> {
    status
        .networks
        .iter()
        .find(|summary| summary.id == network)
        .ok_or_else(|| ApiError::not_found("this device is not in that network"))
}

/// Sends a change only an admin may make.
///
/// The daemon refuses a member with `InvalidArgument`; that refusal becomes 403
/// `permission_denied` here, so the page can say "Only admins can do this".
pub(crate) async fn as_admin(
    daemon: &Daemon,
    network: NetworkId,
    request: Request,
) -> Result<Reply, ApiError> {
    match daemon.ask(request).await {
        Ok(answer) => Ok(answer),
        Err(error) if error.code() == ApiErrorCode::InvalidArgument => {
            let admin = status_of(daemon)
                .await
                .ok()
                .and_then(|status| Some(summary(&status, network).ok()?.admin));
            if admin == Some(false) {
                Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    ApiErrorCode::PermissionDenied,
                    error.body.message,
                ))
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

/// The journal the daemon still holds, oldest first.
pub(crate) async fn history(daemon: &Daemon) -> Result<Vec<Entry>, ApiError> {
    let (next, mut entries) = daemon.connect().await?.journal(None).await?;
    if next == 0 {
        return Ok(Vec::new());
    }
    let read = async move {
        let mut kept = Vec::new();
        while let Some(entry) = entries.recv().await {
            let entry = entry?;
            let last = entry.seq.saturating_add(1) >= next;
            kept.push(entry);
            if last {
                break;
            }
        }
        Ok::<_, ApiError>(kept)
    };
    tokio::time::timeout(HISTORY_TIME, read)
        .await
        .map_err(|_| ApiError::unexpected())?
}

/// The 409 for a change to something another device already settled.
pub(crate) fn settled(resolution: Option<Resolution>, at_ms: u64, op: Option<String>) -> ApiError {
    ApiError::conflict(Conflict {
        resolved_by: String::new(),
        resolution,
        at_ms,
        op,
    })
}

/// The text as a QR code, or `None` when it is too long for one.
///
/// A config for a large network can pass the QR limit; the page still gets the text.
pub(crate) fn qr_svg(text: &str) -> Option<String> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color<'_>>()
            .min_dimensions(240, 240)
            .quiet_zone(false)
            .build(),
    )
}
