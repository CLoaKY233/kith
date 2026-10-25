#![expect(
    clippy::needless_pass_by_value,
    clippy::unused_async,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use kith_ipc::{ErrorCode, Event, IpcServer, PROTOCOL_VERSION, PeerCred, Request, Response};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::types::{Action, Command, NodeError};

/// Events buffered per watcher before the slowest one starts missing them.
pub(crate) const EVENT_BUFFER: usize = 256;

/// Accepts IPC clients until the loop stops; each connection is served on its own task.
pub(crate) async fn serve(
    server: IpcServer,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    daemon_uid: u32,
) -> Result<(), NodeError> {
    Err(NodeError::Pending("ipc: serve"))
}

/// Answers requests that never reach the loop: `Hello`, and `Watch` outside a connection.
pub(crate) fn answer_locally(request: &Request) -> Option<Response> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "every other request reaches the loop"
    )]
    match request {
        Request::Hello { .. } => Some(Response::Hello {
            protocol: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        }),
        Request::Watch => Some(error_response(&NodeError::InvalidArgument(
            "watch needs an IPC connection".to_owned(),
        ))),
        _ => None,
    }
}

/// Decodes and validates a request into a loop action.
pub(crate) fn translate(request: Request) -> Result<Action, NodeError> {
    Err(NodeError::Pending("ipc: translate"))
}

/// Whether a request only reads state, so any local user may send it.
pub(crate) const fn is_read_only(request: &Request) -> bool {
    matches!(
        request,
        Request::Hello { .. } | Request::Status | Request::Peers { .. } | Request::Watch
    )
}

/// Whether a client may change state: root or the daemon's own uid.
pub(crate) fn may_change_state(peer: Option<PeerCred>, daemon_uid: u32) -> bool {
    peer.is_some_and(|peer| peer.uid == 0 || peer.uid == daemon_uid)
}

/// Sends an action to the loop and waits for its response.
pub(crate) async fn dispatch(commands: &mpsc::Sender<Command>, action: Action) -> Response {
    let (reply, response) = oneshot::channel();
    if commands
        .send(Command::Local { action, reply })
        .await
        .is_err()
    {
        return error_response(&NodeError::Stopped);
    }
    match response.await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error_response(&error),
        Err(_) => error_response(&NodeError::Stopped),
    }
}

/// Maps a node error to the IPC error response.
pub(crate) fn error_response(error: &NodeError) -> Response {
    let code = match error {
        NodeError::NotFound(_) => ErrorCode::NotFound,
        NodeError::InvalidArgument(_) | NodeError::Invite(_) => ErrorCode::InvalidArgument,
        NodeError::Pending(_)
        | NodeError::Poisoned
        | NodeError::Channel(_)
        | NodeError::Stopped
        | NodeError::Config(_)
        | NodeError::Store(_)
        | NodeError::Identity(_)
        | NodeError::Ledger(_)
        | NodeError::Control(_)
        | NodeError::Zone(_)
        | NodeError::Dataplane(_)
        | NodeError::Tun(_)
        | NodeError::Udp(_)
        | NodeError::Sys(_)
        | NodeError::Ipc(_)
        | NodeError::Relay(_)
        | NodeError::Io(_) => ErrorCode::Internal,
    };
    Response::Error {
        code,
        message: error.to_string(),
    }
}
