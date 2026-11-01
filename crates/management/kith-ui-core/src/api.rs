//! `/api/...`: reads and changes, each one or a few IPC requests to the daemon.

pub(crate) mod blocked;
pub(crate) mod devices;
pub(crate) mod exits;
pub(crate) mod invites;
pub(crate) mod joins;
pub(crate) mod peers;
pub(crate) mod phones;
pub(crate) mod records;
pub(crate) mod status;
pub(crate) mod traffic;
pub(crate) mod validate;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kith_ipc::{ErrorCode, IpcError};
use kith_types::{NetworkId, NodeId};
use qrcode::QrCode;
use qrcode::render::svg;
use serde_json::{Value, json};

use crate::wire::{ApiErrorBody, ApiErrorCode, Outcome, OutcomeState};

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

    pub(crate) fn bad(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidArgument,
            message,
        )
    }

    /// For an endpoint the contract names but this server cannot do yet.
    pub(crate) fn not_implemented(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            ApiErrorCode::NotImplemented,
            format!("{what} is not available yet"),
        )
    }

    /// For a change asked of the fixture server, which only replays.
    pub(crate) fn fixture() -> Self {
        Self::not_implemented("changing a recorded network")
    }

    pub(crate) fn unexpected() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            "the daemon gave an unexpected answer",
        )
    }
}

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

/// The answer to a change that was signed or applied.
pub(crate) fn accepted() -> Json<Value> {
    Json(json!(Outcome {
        state: OutcomeState::Accepted,
        op: None,
    }))
}

pub(crate) fn network_id(text: &str) -> Result<NetworkId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a network id"))
}

pub(crate) fn node_id(text: &str) -> Result<NodeId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a device id"))
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
