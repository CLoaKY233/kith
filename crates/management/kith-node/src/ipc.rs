use core::time::Duration;

use kith_config::WgQuickConfig;
use kith_control::records::{DNS_PREFIX, DnsData, DnsRecords, Record};
use kith_identity::{InviteToken, RecoveryCode, RequestCode};
use kith_ipc::{
    AccountRole, AuthorityChange, Connection, DnsRecord, Entry, ErrorCode, Event, IpcServer,
    PROTOCOL_VERSION, PasskeyCredential, PeerCred, Request, Response,
};
use kith_ledger::{Approval, Assertion, Credential, OpBody, Role, UnsignedOp};
use kith_naming::DEFAULT_DEVICE_TTL;
use kith_policy::Policy;
use kith_types::NetworkId;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};

use crate::blocked::allow_rule;
use crate::journal::Journal;
use crate::types::{Action, Command, NodeError};

/// Events buffered per watcher before the slowest one starts missing them.
pub(crate) const EVENT_BUFFER: usize = 256;

/// Accepts IPC clients until the loop stops; each connection is served on its own task.
pub(crate) async fn serve(
    server: IpcServer,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    journal: Journal,
    operators: Operators,
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
        let journal = journal.clone();
        tokio::spawn(async move {
            if let Err(error) =
                serve_connection(connection, commands, events, journal, operators).await
            {
                debug!(%error, "ipc connection ended");
            }
        });
    }
}

async fn serve_connection(
    mut connection: Connection,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    journal: Journal,
    operators: Operators,
) -> Result<(), NodeError> {
    while let Some(request) = connection.next_request().await? {
        if matches!(request, Request::Watch) {
            let watcher = events.subscribe();
            connection.respond(Response::Watching).await?;
            return watch(connection, watcher).await;
        }
        if let Request::Journal { since } = request {
            let (backlog, live, next) = journal.subscribe(since);
            connection.respond(Response::Journaling { next }).await?;
            return follow(connection, backlog, live).await;
        }
        let response = if !is_read_only(&request) && !operators.may_change_state(connection.peer())
        {
            Response::Error {
                code: ErrorCode::PermissionDenied,
                message: "only root, the daemon's user or the operator group may change state"
                    .to_owned(),
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

/// Sends `backlog`, then every entry from `live`; a client too slow to keep up skips the
/// entries it missed and sees the gap in their sequence numbers.
async fn follow(
    mut connection: Connection,
    backlog: Vec<Entry>,
    mut live: broadcast::Receiver<Entry>,
) -> Result<(), NodeError> {
    let mut last = None;
    for entry in backlog {
        last = Some(entry.seq);
        connection.send_entry(entry).await?;
    }
    loop {
        let entry = match live.recv().await {
            Ok(entry) => entry,
            // Not logged: the log is copied into this journal, so a slow reader would add
            // an entry for every time it fell behind and fall further behind.
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return Ok(()),
        };
        if last.is_some_and(|seq| entry.seq <= seq) {
            continue;
        }
        last = Some(entry.seq);
        connection.send_entry(entry).await?;
    }
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
        Request::Records { network } => Action::Records { network },
        Request::Traffic => Action::Traffic,
        Request::Topology { network } => Action::Topology { network },
        request @ (Request::Blocked { .. } | Request::AllowBlocked { .. }) => blocked(request)?,
        Request::SetRecord {
            network,
            name,
            record,
        } => set_record(network, &name, record)?,
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
        Request::RemoveDevice { network, node } => Action::RemoveDevice { network, node },
        Request::AdvertiseRelay {
            network,
            name,
            address,
            node,
        } => Action::AdvertiseRelay {
            network,
            name: relay_name(&name)?,
            address: non_empty("relay address", address)?,
            node,
        },
        Request::RemoveRelay { network, name } => Action::RemoveRelay {
            network,
            name: relay_name(&name)?,
        },
        Request::SetExitGrant {
            network,
            node,
            allowed,
        } => Action::SetExitGrant {
            network,
            node,
            allowed,
        },
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
            tags,
        } => Action::ExportWireGuard {
            network,
            device_name: non_empty("device name", device_name)?,
            tags,
        },
        other @ (Request::Journal { .. } | _) => return admission_action(other),
    };
    Ok(action)
}

fn admission_action(request: Request) -> Result<Action, NodeError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "every request translate does not know is unsupported, Journal included"
    )]
    Ok(match request {
        Request::RequestCode { network } => Action::RequestCode { network },
        Request::RequestJoin {
            request_code,
            device_name,
        } => Action::RequestJoin {
            code: Box::new(RequestCode::parse(request_code.trim()).map_err(|error| {
                NodeError::InvalidArgument(format!("not a request code: {error}"))
            })?),
            device_name: non_empty("device name", device_name)?,
        },
        Request::PendingJoins => Action::PendingJoins,
        Request::ApproveJoin { network, node } => Action::ApproveJoin { network, node },
        Request::DenyJoin { network, node } => Action::DenyJoin { network, node },
        Request::RenameDevice {
            network,
            node,
            name,
        } => Action::RenameDevice {
            network,
            node,
            name: device_name(name)?,
        },
        Request::ListInvites { network } => Action::ListInvites { network },
        Request::RevokeInvite { network, invite } => Action::RevokeInvite {
            network,
            invite: non_empty("invite id", invite)?,
        },
        Request::GetPolicy { network } => Action::GetPolicy { network },
        Request::ListRelays { network } => Action::ListRelays { network },
        Request::ListExits { network } => Action::ListExits { network },
        Request::UseImport { network, active } => Action::UseImport { network, active },
        Request::RemoveImport { network } => Action::RemoveImport { network },
        other @ (Request::Accounts { .. }
        | Request::AddAccount { .. }
        | Request::AssignDevice { .. }
        | Request::PrepareAuthority { .. }
        | Request::SubmitAuthority { .. }
        | Request::RecoverAuthority { .. }
        | Request::RecoveryKey { .. }
        | Request::AsActor { .. }) => people_action(other)?,
        _ => return Err(NodeError::InvalidArgument("unsupported request".to_owned())),
    })
}

/// The requests about people: accounts, approvals, the recovery key, acting for someone.
fn people_action(request: Request) -> Result<Action, NodeError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "admission_action passes only the requests about people"
    )]
    Ok(match request {
        Request::Accounts { network } => Action::Accounts { network },
        Request::AddAccount {
            network,
            name,
            role,
            credential,
        } => {
            if role != AccountRole::Owner {
                return Err(NodeError::InvalidArgument(
                    "this device adds only the network's first owner; add everyone else with \
                     an owner's or admin's passkey"
                        .to_owned(),
                ));
            }
            Action::AddAccount {
                network,
                name: person_name(name)?,
                credential: passkey(credential),
            }
        }
        Request::AssignDevice {
            network,
            device,
            account,
        } => Action::AssignDevice {
            network,
            device,
            account,
        },
        Request::PrepareAuthority { network, change } => Action::PrepareAuthority {
            network,
            change: Box::new(authority_change(change)?),
        },
        Request::SubmitAuthority {
            network,
            op_bytes,
            assertion,
        } => Action::SubmitAuthority {
            network,
            draft: Box::new(prepared(&op_bytes)?),
            approval: Box::new(Approval::Passkey(Assertion {
                account: assertion.account,
                credential_id: assertion.credential_id,
                authenticator_data: assertion.authenticator_data,
                client_data_json: assertion.client_data_json,
                signature: assertion.signature,
            })),
        },
        Request::RecoverAuthority {
            network,
            op_bytes,
            words,
        } => {
            let draft = prepared(&op_bytes)?;
            let code = RecoveryCode::parse(&words)
                .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
            let challenge = draft
                .id()
                .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
            Action::SubmitAuthority {
                network,
                draft: Box::new(draft),
                approval: Box::new(Approval::recovery(&code.signing_key(), &challenge)),
            }
        }
        Request::RecoveryKey { network } => Action::RecoveryKey { network },
        Request::AsActor { actor, request } => {
            if matches!(*request, Request::AsActor { .. }) {
                return Err(NodeError::InvalidArgument(
                    "a request acts for one person at most".to_owned(),
                ));
            }
            Action::AsActor {
                actor,
                action: Box::new(translate(*request)?),
            }
        }
        _ => {
            return Err(NodeError::InvalidArgument(
                "not a people request".to_owned(),
            ));
        }
    })
}

const fn role(role: AccountRole) -> Role {
    match role {
        AccountRole::Owner => Role::Owner,
        AccountRole::Admin => Role::Admin,
        AccountRole::Member => Role::Member,
    }
}

fn passkey(credential: PasskeyCredential) -> Credential {
    Credential {
        id: credential.credential_id,
        public_key: credential.public_key,
        rp_id: credential.rp_id,
    }
}

fn authority_change(change: AuthorityChange) -> Result<OpBody, NodeError> {
    Ok(match change {
        AuthorityChange::AddAccount {
            name,
            role: wanted,
            credential,
        } => OpBody::AddAccount {
            name: person_name(name)?,
            role: role(wanted),
            credential: passkey(credential),
        },
        AuthorityChange::RemoveAccount { account } => OpBody::RemoveAccount { account },
        AuthorityChange::SetRole {
            account,
            role: wanted,
        } => OpBody::SetRole {
            account,
            role: role(wanted),
        },
        AuthorityChange::AuthorizeConsole { device } => OpBody::AuthorizeConsole { device },
        AuthorityChange::RevokeConsole { device } => OpBody::RevokeConsole { device },
    })
}

/// Decodes the op bytes that `PrepareAuthority` handed out.
fn prepared(op_bytes: &[u8]) -> Result<UnsignedOp, NodeError> {
    UnsignedOp::from_bytes(op_bytes)
        .map_err(|error| NodeError::InvalidArgument(format!("not a prepared change: {error}")))
}

fn blocked(request: Request) -> Result<Action, NodeError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "translate passes only the two blocked requests"
    )]
    match request {
        Request::Blocked { network } => Ok(Action::Blocked { network }),
        Request::AllowBlocked {
            network,
            from,
            to,
            protocol,
            port,
        } => Ok(Action::AllowBlocked {
            network,
            rule: Box::new(allow_rule(from, to, &protocol, port)?),
        }),
        _ => Err(NodeError::InvalidArgument(
            "not a blocked request".to_owned(),
        )),
    }
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

/// A device name as joins take it: not empty, and short enough for the ledger.
fn device_name(name: String) -> Result<String, NodeError> {
    let name = non_empty("device name", name)?;
    if name.len() > kith_ledger::MAX_NAME_LEN {
        return Err(NodeError::InvalidArgument(format!(
            "device name is longer than {} bytes",
            kith_ledger::MAX_NAME_LEN
        )));
    }
    Ok(name)
}

/// A person's name: not empty, and short enough for the ledger.
fn person_name(name: String) -> Result<String, NodeError> {
    let name = non_empty("name", name)?;
    if name.len() > kith_ledger::MAX_NAME_LEN {
        return Err(NodeError::InvalidArgument(format!(
            "name is longer than {} bytes",
            kith_ledger::MAX_NAME_LEN
        )));
    }
    Ok(name)
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

fn set_record(network: NetworkId, name: &str, record: DnsRecord) -> Result<Action, NodeError> {
    let record = dns_record(name, record)?;
    let value = record
        .encode()
        .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
    Ok(Action::SetRecord {
        network,
        key: record.key(),
        value,
    })
}

fn relay_name(name: &str) -> Result<String, NodeError> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(NodeError::InvalidArgument("relay name is empty".to_owned()));
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
        Request::Hello { .. }
            | Request::Status
            | Request::Peers { .. }
            | Request::Records { .. }
            | Request::Journal { .. }
            | Request::Traffic
            | Request::Topology { .. }
            | Request::Blocked { .. }
            | Request::Watch
            | Request::PendingJoins
            | Request::RequestCode { .. }
            | Request::ListInvites { .. }
            | Request::GetPolicy { .. }
            | Request::ListRelays { .. }
            | Request::ListExits { .. }
            | Request::Accounts { .. }
    )
}

/// Who may change state over IPC.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Operators {
    /// The daemon's own user.
    pub(crate) daemon_uid: u32,
    /// The operator group, if it exists.
    pub(crate) group: Option<u32>,
}

impl Operators {
    /// Whether a client may change state: root, the daemon's user, or an operator.
    pub(crate) fn may_change_state(self, peer: Option<PeerCred>) -> bool {
        peer.is_some_and(|peer| {
            peer.uid == 0
                || peer.uid == self.daemon_uid
                || self
                    .group
                    .is_some_and(|gid| kith_sys::user_in_group(peer.uid, gid))
        })
    }
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
        NodeError::InvalidArgument(_) | NodeError::Invite(_) | NodeError::PortInUse { .. } => {
            ErrorCode::InvalidArgument
        }
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
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    Response::Error { code, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn peer(uid: u32) -> PeerCred {
        PeerCred {
            uid,
            gid: uid,
            pid: None,
        }
    }

    #[test]
    fn root_the_daemon_user_and_operators_may_change_state() {
        let me = kith_sys::effective_uid().unwrap_or(0);
        let open = Operators {
            daemon_uid: 0,
            group: None,
        };
        assert!(open.may_change_state(Some(peer(0))));
        assert!(
            !open.may_change_state(Some(peer(me.max(1)))),
            "no group, no operators"
        );
        assert!(!open.may_change_state(None), "unknown peers are refused");

        let as_user = Operators {
            daemon_uid: 4242,
            group: None,
        };
        assert!(as_user.may_change_state(Some(peer(4242))));

        let root_group = Operators {
            daemon_uid: 0,
            group: Some(0),
        };
        assert!(root_group.may_change_state(Some(peer(0))));
        assert!(
            !root_group.may_change_state(Some(peer(u32::MAX - 7))),
            "an unknown user is no operator"
        );
    }
}
