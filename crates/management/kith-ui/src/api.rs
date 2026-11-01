//! `/api/...`: reads and actions, each one IPC request to the daemon.

use core::net::{Ipv4Addr, Ipv6Addr};

use bytes::Bytes;
use http_body_util::{BodyExt as _, Limited};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use kith_ipc::{DnsRecord, ErrorCode, IpcClient, IpcError, Request as Ipc, Response as Answer};
use kith_types::{NetworkId, NodeId};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Body, State, events, json, session};

/// The largest request body accepted; actions carry a few short fields.
const MAX_BODY: usize = 64 * 1024;

/// A failed request, answered as `{"error": message}`.
#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
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
                let status = match code {
                    ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
                    ErrorCode::NotFound => StatusCode::NOT_FOUND,
                    ErrorCode::InvalidArgument => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                Self::new(status, message)
            }
            other => Self::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

pub(crate) fn error(status: StatusCode, message: &str) -> Response<Body> {
    session::full(
        status,
        "application/json",
        json!({ "error": message }).to_string(),
    )
}

fn ok(value: &Value) -> Response<Body> {
    session::full(StatusCode::OK, "application/json", value.to_string())
}

pub(crate) async fn connect(state: &State) -> Result<IpcClient, ApiError> {
    IpcClient::connect(&state.socket).await.map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "cannot reach the daemon at {} ({error}); is kith running?",
                state.socket.display()
            ),
        )
    })
}

async fn ask(state: &State, request: Ipc) -> Result<Answer, ApiError> {
    Ok(connect(state).await?.request(request).await?)
}

fn unexpected() -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the daemon gave an unexpected answer",
    )
}

fn network(text: &str) -> Result<NetworkId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a network id"))
}

fn node(text: &str) -> Result<NodeId, ApiError> {
    text.parse().map_err(|_| ApiError::bad("not a device id"))
}

/// Decodes `%XX` escapes in a path segment, such as a record name.
fn unescape(segment: &str) -> Result<String, ApiError> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let digits = bytes
                .get(index + 1..index + 3)
                .and_then(|pair| core::str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| ApiError::bad("bad escape in the path"))?;
            out.push(digits);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ApiError::bad("the path is not UTF-8"))
}

async fn body<T: DeserializeOwned>(request: Request<Incoming>) -> Result<T, ApiError> {
    let bytes: Bytes = Limited::new(request.into_body(), MAX_BODY)
        .collect()
        .await
        .map_err(|_| ApiError::bad("the request body is too large or was cut off"))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|error| ApiError::bad(format!("bad JSON: {error}")))
}

fn qr_svg(text: &str) -> Result<String, ApiError> {
    let code = QrCode::new(text.as_bytes()).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot make a QR code: {error}"),
        )
    })?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(240, 240)
        .build())
}

fn config(text: &str) -> Result<Value, ApiError> {
    Ok(json!({ "config": text, "qr_svg": qr_svg(text)? }))
}

#[derive(Deserialize)]
struct InviteIn {
    expires_secs: u64,
    /// `None` is unlimited.
    uses: Option<u32>,
}

#[derive(Deserialize)]
struct PhoneIn {
    name: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct RecordIn {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Deserialize)]
struct ExitIn {
    network: Option<String>,
    node: Option<String>,
}

#[derive(Deserialize)]
struct GrantIn {
    allowed: bool,
}

fn record(kind: &str, value: &str) -> Result<DnsRecord, ApiError> {
    let invalid = || ApiError::bad(format!("{value} is not a valid {kind} value"));
    match kind.to_ascii_uppercase().as_str() {
        "A" => value
            .parse::<Ipv4Addr>()
            .map(DnsRecord::A)
            .map_err(|_| invalid()),
        "AAAA" => value
            .parse::<Ipv6Addr>()
            .map(DnsRecord::Aaaa)
            .map_err(|_| invalid()),
        "CNAME" => Ok(DnsRecord::Cname(value.to_owned())),
        "TXT" => Ok(DnsRecord::Txt(value.to_owned())),
        _ => Err(ApiError::bad("the type must be A, AAAA, CNAME or TXT")),
    }
}

async fn read(state: &State, request: Ipc) -> Result<Value, ApiError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "each read has one right answer"
    )]
    match ask(state, request).await? {
        Answer::Status(status) => Ok(json::status(&status)?),
        Answer::Peers(peers) => Ok(json::to_value(&peers)?),
        Answer::Records(records) => Ok(Value::Array(records.iter().map(json::record).collect())),
        Answer::Traffic(traffic) => Ok(json::to_value(&traffic)?),
        _ => Err(unexpected()),
    }
}

/// Sends an action whose answer carries nothing the page needs.
async fn act(state: &State, request: Ipc) -> Result<Value, ApiError> {
    ask(state, request).await?;
    Ok(json!({}))
}

async fn invite(state: &State, network: NetworkId, input: InviteIn) -> Result<Value, ApiError> {
    let Answer::Invite { invite_link } = ask(
        state,
        Ipc::CreateInvite {
            network,
            expires_in_secs: input.expires_secs,
            max_uses: input.uses,
        },
    )
    .await?
    else {
        return Err(unexpected());
    };
    Ok(json!({ "qr_svg": qr_svg(&invite_link)?, "link": invite_link }))
}

async fn phone(state: &State, network: NetworkId, input: PhoneIn) -> Result<Value, ApiError> {
    let Answer::WireGuardConfig {
        device,
        mesh,
        everything,
        ..
    } = ask(
        state,
        Ipc::ExportWireGuard {
            network,
            device_name: input.name,
            tags: input.tags,
        },
    )
    .await?
    else {
        return Err(unexpected());
    };
    let everything = match everything {
        Some(text) => config(&text)?,
        None => Value::Null,
    };
    Ok(json!({ "name": device, "device": config(&mesh)?, "everything": everything }))
}

/// Uses an exit node, or stops with `network: null`, which needs some member network.
async fn exit(state: &State, input: ExitIn) -> Result<Value, ApiError> {
    let network = match (&input.network, &input.node) {
        (Some(text), _) => network(text)?,
        (None, Some(_)) => return Err(ApiError::bad("an exit node needs its network")),
        (None, None) => {
            let Answer::Status(status) = ask(state, Ipc::Status).await? else {
                return Err(unexpected());
            };
            status
                .networks
                .first()
                .map(|summary| summary.id)
                .ok_or_else(|| {
                    ApiError::new(StatusCode::CONFLICT, "this device is in no network")
                })?
        }
    };
    let node = input.node.as_deref().map(node).transpose()?;
    act(state, Ipc::UseExit { network, node }).await
}

async fn route(
    state: &State,
    route: &str,
    query: Option<&str>,
    request: Request<Incoming>,
) -> Result<Response<Body>, ApiError> {
    let segments: Vec<&str> = route.split('/').collect();
    let get = request.method() == Method::GET;
    let post = request.method() == Method::POST;
    let value = match segments.as_slice() {
        ["status"] if get => read(state, Ipc::Status).await?,
        ["traffic"] if get => read(state, Ipc::Traffic).await?,
        ["events"] if get => return events::stream(state, request.headers(), query).await,
        ["exit"] if post => exit(state, body(request).await?).await?,
        ["networks", id, "peers"] if get => {
            read(
                state,
                Ipc::Peers {
                    network: network(id)?,
                },
            )
            .await?
        }
        ["networks", id, "records"] if get => {
            read(
                state,
                Ipc::Records {
                    network: network(id)?,
                },
            )
            .await?
        }
        ["networks", id, "records"] if post => {
            let network = network(id)?;
            let input: RecordIn = body(request).await?;
            let record = record(&input.kind, &input.value)?;
            act(
                state,
                Ipc::SetRecord {
                    network,
                    name: input.name,
                    record,
                },
            )
            .await?
        }
        ["networks", id, "records", name, "delete"] if post => {
            let request = Ipc::DeleteRecord {
                network: network(id)?,
                name: unescape(name)?,
            };
            act(state, request).await?
        }
        ["networks", id, "invites"] if post => {
            let network = network(id)?;
            invite(state, network, body(request).await?).await?
        }
        ["networks", id, "phones"] if post => {
            let network = network(id)?;
            phone(state, network, body(request).await?).await?
        }
        ["networks", id, "devices", device, "remove"] if post => {
            let request = Ipc::RemoveDevice {
                network: network(id)?,
                node: node(device)?,
            };
            act(state, request).await?
        }
        ["networks", id, "exits", device] if post => {
            let (network, node) = (network(id)?, node(device)?);
            let input: GrantIn = body(request).await?;
            act(
                state,
                Ipc::SetExitGrant {
                    network,
                    node,
                    allowed: input.allowed,
                },
            )
            .await?
        }
        _ => return Err(ApiError::new(StatusCode::NOT_FOUND, "no such API path")),
    };
    Ok(ok(&value))
}

pub(crate) async fn handle(
    state: &State,
    path: &str,
    query: Option<&str>,
    request: Request<Incoming>,
) -> Response<Body> {
    match route(state, path, query, request).await {
        Ok(response) => response,
        Err(failure) => error(failure.status, &failure.message),
    }
}
