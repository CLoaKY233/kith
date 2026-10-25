use core::time::Duration;

use kith_config::WgQuickConfig;
use kith_control::records::{DNS_PREFIX, DnsData, DnsRecords, Record};
use kith_identity::InviteToken;
use kith_ipc::{
    Connection, DnsRecord, ErrorCode, Event, IpcServer, PROTOCOL_VERSION, PeerCred, Request,
    Response,
};
use kith_naming::DEFAULT_DEVICE_TTL;
use kith_policy::Policy;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};

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
    loop {
        let connection = tokio::select! {
            () = commands.closed() => return Ok(()),
            accepted = server.accept() => match accepted {
                Ok(connection) => connection,
                Err(error) => {
                    warn!(%error, "ipc accept failed");
                    continue;
                }
            },
        };
        let commands = commands.clone();
        let events = events.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(connection, commands, events, daemon_uid).await {
                debug!(%error, "ipc connection ended");
            }
        });
    }
}

async fn serve_connection(
    mut connection: Connection,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    daemon_uid: u32,
) -> Result<(), NodeError> {
    while let Some(request) = connection.next_request().await? {
        if matches!(request, Request::Watch) {
            let watcher = events.subscribe();
            connection.respond(Response::Watching).await?;
            return watch(connection, watcher).await;
        }
        let response =
            if !is_read_only(&request) && !may_change_state(connection.peer(), daemon_uid) {
                Response::Error {
                    code: ErrorCode::PermissionDenied,
                    message: "only root or the daemon's user may change state".to_owned(),
                }
            } else if let Some(response) = answer_locally(&request) {
                response
            } else {
                match translate(request) {
                    Ok(action) => dispatch(&commands, action).await,
                    Err(error) => error_response(&error),
                }
            };
        connection.respond(response).await?;
    }
    Ok(())
}

async fn watch(
    mut connection: Connection,
    mut events: broadcast::Receiver<Event>,
) -> Result<(), NodeError> {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(missed)) => Event::Warning {
                message: format!("{missed} events were dropped because the client was slow"),
            },
            Err(RecvError::Closed) => return Ok(()),
        };
        connection.send_event(event).await?;
    }
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
    let action = match request {
        Request::Hello { .. } | Request::Watch => {
            return Err(NodeError::InvalidArgument(
                "this request is answered by the connection".to_owned(),
            ));
        }
        Request::Status => Action::Status,
        Request::CreateNetwork { name, device_name } => Action::CreateNetwork {
            name: non_empty("network name", name)?,
            device_name: non_empty("device name", device_name)?,
        },
        Request::CreateInvite {
            network,
            expires_in_secs,
            max_uses,
        } => Action::CreateInvite {
            network,
            expires_in: invite_lifetime(expires_in_secs)?,
            max_uses: invite_uses(max_uses)?,
        },
        Request::Join {
            invite_link,
            device_name,
        } => Action::Join {
            token: Box::new(InviteToken::parse(invite_link.trim())?),
            device_name: non_empty("device name", device_name)?,
        },
        Request::Leave { network } => Action::Leave { network },
        Request::Peers { network } => Action::Peers { network },
        Request::SetRecord {
            network,
            name,
            record,
        } => {
            let record = dns_record(&name, record)?;
            let value = record
                .encode()
                .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
            Action::SetRecord {
                network,
                key: record.key(),
                value,
            }
        }
        Request::DeleteRecord { network, name } => Action::DeleteRecord {
            network,
            key: format!("{DNS_PREFIX}{}", dns_name(&name)?),
        },
        Request::SetPolicy {
            network,
            policy_toml,
        } => Action::SetPolicy {
            network,
            policy: Box::new(
                toml::from_str::<Policy>(&policy_toml)
                    .map_err(|error| NodeError::InvalidArgument(format!("policy: {error}")))?,
            ),
        },
        Request::UseExit { network, node } => Action::UseExit { network, node },
        Request::ServerMode { exit, relay } => Action::ServerMode { exit, relay },
        Request::ImportWireGuard { name, conf_text } => Action::ImportWireGuard {
            name: non_empty("import name", name)?,
            config: Box::new(
                WgQuickConfig::parse(&conf_text)
                    .map_err(|error| NodeError::InvalidArgument(format!("wg-quick: {error}")))?,
            ),
        },
        Request::ExportWireGuard {
            network,
            device_name,
        } => Action::ExportWireGuard {
            network,
            device_name: non_empty("device name", device_name)?,
        },
        _ => {
            return Err(NodeError::InvalidArgument("unsupported request".to_owned()));
        }
    };
    Ok(action)
}

fn non_empty(what: &str, value: String) -> Result<String, NodeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(NodeError::InvalidArgument(format!("{what} is empty")));
    }
    Ok(if trimmed.len() == value.len() {
        value
    } else {
        trimmed.to_owned()
    })
}

fn invite_lifetime(secs: u64) -> Result<Duration, NodeError> {
    if secs == 0 {
        return Err(NodeError::InvalidArgument(
            "invite lifetime must be positive".to_owned(),
        ));
    }
    Ok(Duration::from_secs(secs))
}

fn invite_uses(max_uses: Option<u32>) -> Result<u16, NodeError> {
    match max_uses {
        None => Ok(u16::MAX),
        Some(uses) => u16::try_from(uses)
            .ok()
            .filter(|uses| *uses > 0)
            .ok_or_else(|| {
                NodeError::InvalidArgument(format!("max uses must be 1 to {}", u16::MAX))
            }),
    }
}

fn dns_name(name: &str) -> Result<String, NodeError> {
    let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return Err(NodeError::InvalidArgument(
            "record name is empty".to_owned(),
        ));
    }
    Ok(name)
}

fn dns_record(name: &str, record: DnsRecord) -> Result<Record, NodeError> {
    let data = match record {
        DnsRecord::A(addr) => DnsData::A(addr),
        DnsRecord::Aaaa(addr) => DnsData::Aaaa(addr),
        DnsRecord::Cname(target) => DnsData::Cname(dns_name(&target)?),
        DnsRecord::Txt(text) => DnsData::Txt(txt_strings(&text)),
        _ => {
            return Err(NodeError::InvalidArgument(
                "unsupported record type".to_owned(),
            ));
        }
    };
    DnsRecords::new(dns_name(name)?, DEFAULT_DEVICE_TTL, vec![data])
        .map(Record::Dns)
        .map_err(|error| NodeError::InvalidArgument(error.to_string()))
}

fn txt_strings(text: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.len().saturating_add(ch.len_utf8()) > kith_naming::MAX_TXT_STRING_LEN {
            strings.push(core::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() || strings.is_empty() {
        strings.push(current);
    }
    strings
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
        NodeError::Poisoned
        | NodeError::Channel(_)
        | NodeError::Stopped
        | NodeError::NoMemoryTun
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
