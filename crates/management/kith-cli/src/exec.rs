//! Runs a parsed command against the daemon and renders the result.

use core::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use kith_config::{Config, ConfigError};
use kith_ipc::{DnsRecord, IpcClient, IpcError, Request, Response};
use kith_types::{NetworkId, NodeId};

use crate::cli::{Cmd, ConfigCmd, DnsCmd, ExitCmd, Opts, RelayCmd, WgCmd};
use crate::{doctor, render};

/// A failure worth one line on stderr.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<IpcError> for CliError {
    fn from(error: IpcError) -> Self {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only remote errors need unwrapping"
        )]
        match error {
            IpcError::Remote { message, .. } => Self(message),
            other => Self(other.to_string()),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self(format!("cannot write output: {error}"))
    }
}

impl From<ConfigError> for CliError {
    fn from(error: ConfigError) -> Self {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only validation errors need joining"
        )]
        let text = match error {
            ConfigError::Invalid(errors) => errors
                .0
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
            other => other.to_string(),
        };
        Self(text.replace('\n', " "))
    }
}

/// What the daemon knows that turns names into ids.
#[derive(Debug, Default)]
pub(crate) struct Names {
    pub networks: Vec<(NetworkId, String)>,
    /// Each peer's node, unique label and the name it asked for.
    pub peers: Vec<(NodeId, String, String)>,
    pub hostname: String,
}

impl Names {
    fn network(&self, text: &str) -> Result<NetworkId, CliError> {
        if let Ok(id) = text.parse() {
            return Ok(id);
        }
        let mut matches = self.networks.iter().filter(|(id, name)| {
            name == text || (text.len() >= 8 && id.to_string().starts_with(text))
        });
        match (matches.next(), matches.next()) {
            (Some((id, _)), None) => Ok(*id),
            (Some(_), Some(_)) => Err(CliError(format!(
                "several networks are named `{text}`; use its id from `kith status`"
            ))),
            (None, _) => Err(CliError(format!(
                "no network named `{text}`; see `kith status`"
            ))),
        }
    }

    pub(crate) fn node(&self, text: &str) -> Result<NodeId, CliError> {
        if let Ok(id) = text.parse() {
            return Ok(id);
        }
        if let Some((id, _, _)) = self.peers.iter().find(|(_, label, _)| label == text) {
            return Ok(*id);
        }
        let named: Vec<&str> = self
            .peers
            .iter()
            .filter(|(_, _, asked)| asked == text)
            .map(|(_, label, _)| label.as_str())
            .collect();
        match named.as_slice() {
            [label] => self.node(label),
            [] => Err(CliError(format!(
                "no peer named `{text}`; see `kith peers`"
            ))),
            labels => Err(CliError(format!(
                "several peers are named `{text}`; use one of: {}",
                labels.join(", ")
            ))),
        }
    }

    fn only_network(&self) -> Result<NetworkId, CliError> {
        match self.networks.as_slice() {
            [(id, _)] => Ok(*id),
            [] => Err(CliError("this device is in no network".to_owned())),
            _ => Err(CliError(
                "this device is in several networks; name one".to_owned(),
            )),
        }
    }

    fn device(&self, given: Option<&String>) -> String {
        given.cloned().unwrap_or_else(|| self.hostname.clone())
    }
}

/// Builds the request for a command that is a single request/response exchange.
pub(crate) fn to_request(cmd: &Cmd, names: &Names) -> Result<Request, CliError> {
    Ok(match cmd {
        Cmd::Status { .. } => Request::Status,
        Cmd::Create { device_name, name } => Request::CreateNetwork {
            name: name.clone(),
            device_name: names.device(device_name.as_ref()),
        },
        Cmd::Invite {
            expires,
            uses,
            network,
        } => Request::CreateInvite {
            network: names.network(network)?,
            expires_in_secs: *expires,
            max_uses: *uses,
        },
        Cmd::Join { device_name, link } => Request::Join {
            invite_link: link.clone(),
            device_name: names.device(device_name.as_ref()),
        },
        Cmd::Leave { network } => Request::Leave {
            network: names.network(network)?,
        },
        Cmd::Peers { network, .. } => Request::Peers {
            network: match network {
                Some(network) => names.network(network)?,
                None => names.only_network()?,
            },
        },
        Cmd::Dns(DnsCmd::List { network, .. }) => Request::Records {
            network: match network {
                Some(network) => names.network(network)?,
                None => names.only_network()?,
            },
        },
        Cmd::Dns(DnsCmd::Set {
            network,
            name,
            kind,
            value,
        }) => Request::SetRecord {
            network: names.network(network)?,
            name: name.clone(),
            record: parse_record(kind, value)?,
        },
        Cmd::Dns(DnsCmd::Rm { network, name }) => Request::DeleteRecord {
            network: names.network(network)?,
            name: name.clone(),
        },
        Cmd::Remove { network, device } => Request::RemoveDevice {
            network: names.network(network)?,
            node: names.node(device)?,
        },
        Cmd::Exit(ExitCmd::Use { network, node }) => Request::UseExit {
            network: names.network(network)?,
            node: Some(names.node(node)?),
        },
        Cmd::Exit(ExitCmd::Off { network }) => Request::UseExit {
            network: match network {
                Some(text) => names.network(text)?,
                None => names.only_network()?,
            },
            node: None,
        },
        Cmd::Exit(ExitCmd::Allow { network, node }) => Request::SetExitGrant {
            network: names.network(network)?,
            node: names.node(node)?,
            allowed: true,
        },
        Cmd::Exit(ExitCmd::Deny { network, node }) => Request::SetExitGrant {
            network: names.network(network)?,
            node: names.node(node)?,
            allowed: false,
        },
        Cmd::Relay(RelayCmd::Advertise {
            node,
            network,
            name,
            address,
        }) => Request::AdvertiseRelay {
            network: names.network(network)?,
            name: name.clone(),
            address: address.clone(),
            node: node.as_deref().map(|node| names.node(node)).transpose()?,
        },
        Cmd::Relay(RelayCmd::Remove { network, name }) => Request::RemoveRelay {
            network: names.network(network)?,
            name: name.clone(),
        },
        Cmd::Server { exit, relay } => Request::ServerMode {
            exit: *exit,
            relay: *relay,
        },
        Cmd::Wg(wg) => wg_request(wg, names)?,
        Cmd::Up | Cmd::Down | Cmd::Watch | Cmd::Doctor | Cmd::Config(_) | Cmd::Daemon { .. } => {
            return Err(CliError("not a daemon request".to_owned()));
        }
    })
}

fn wg_request(cmd: &WgCmd, names: &Names) -> Result<Request, CliError> {
    Ok(match cmd {
        WgCmd::Import { name, file } => Request::ImportWireGuard {
            name: name.clone(),
            conf_text: read_wg(file)?,
        },
        WgCmd::Export {
            tags,
            network,
            device_name,
            ..
        } => Request::ExportWireGuard {
            network: names.network(network)?,
            device_name: device_name.clone(),
            tags: tags.clone(),
        },
    })
}

fn parse_record(kind: &str, value: &str) -> Result<DnsRecord, CliError> {
    let bad = |what: &str| CliError(format!("`{value}` is not {what}"));
    Ok(match kind.to_ascii_lowercase().as_str() {
        "a" => DnsRecord::A(value.parse().map_err(|_| bad("an IPv4 address"))?),
        "aaaa" => DnsRecord::Aaaa(value.parse().map_err(|_| bad("an IPv6 address"))?),
        "cname" => DnsRecord::Cname(value.to_owned()),
        "txt" => DnsRecord::Txt(value.to_owned()),
        _ => {
            return Err(CliError(format!(
                "unknown record type `{kind}`; use A, AAAA, CNAME or TXT"
            )));
        }
    })
}

fn read_wg(file: &Path) -> Result<String, CliError> {
    // Parse locally so a broken file fails with a line number before reaching the daemon.
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError(format!("cannot read {}: {e}", file.display())))?;
    kith_config::WgQuickConfig::parse(&text)
        .map_err(|e| CliError(format!("{}: {e}", file.display())))?;
    Ok(text)
}

/// Loads the configuration for `kith daemon` and `kith config show`.
pub(crate) fn load_config(path: Option<&Path>) -> Result<Config, CliError> {
    Ok(match path {
        Some(path) => Config::load(path)?,
        None => Config::load_or_default(&Config::default_path())?,
    })
}

pub(crate) fn socket_path(opts: &Opts) -> Result<PathBuf, CliError> {
    match &opts.socket {
        Some(path) => Ok(path.clone()),
        None => Ok(Config::load_or_default(&Config::default_path())?
            .daemon
            .socket_path),
    }
}

async fn connect(path: &Path) -> Result<IpcClient, CliError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "only socket errors get a hint"
    )]
    IpcClient::connect(path).await.map_err(|error| match error {
        IpcError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => CliError(format!(
            "no permission to use {}; run with sudo, or join the operator group (admin on macOS, kith on Linux)",
            path.display()
        )),
        IpcError::Io(io) => CliError(format!(
            "cannot reach the daemon at {} ({io}); is kith daemon running?",
            path.display()
        )),
        other => other.into(),
    })
}

/// Runs every command except `daemon`, writing human or JSON output to `out`.
pub(crate) async fn run(opts: &Opts, out: &mut impl Write) -> Result<(), CliError> {
    match &opts.cmd {
        Cmd::Config(ConfigCmd::Check { path }) => {
            let path = path.clone().unwrap_or_else(Config::default_path);
            match Config::load(&path) {
                Err(ConfigError::Read { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    writeln!(out, "{}: no such file; defaults apply", path.display())?;
                }
                result => {
                    result?;
                    writeln!(out, "{}: ok", path.display())?;
                }
            }
            return Ok(());
        }
        Cmd::Config(ConfigCmd::Show { path }) => {
            out.write_all(load_config(path.as_deref())?.to_toml_string()?.as_bytes())?;
            return Ok(());
        }
        Cmd::Daemon { .. } | Cmd::Up | Cmd::Down => {
            return Err(CliError("not handled here".to_owned()));
        }
        Cmd::Status { .. }
        | Cmd::Create { .. }
        | Cmd::Invite { .. }
        | Cmd::Join { .. }
        | Cmd::Leave { .. }
        | Cmd::Remove { .. }
        | Cmd::Peers { .. }
        | Cmd::Dns(_)
        | Cmd::Exit(_)
        | Cmd::Relay(_)
        | Cmd::Server { .. }
        | Cmd::Wg(_)
        | Cmd::Watch => {}
        Cmd::Doctor => return doctor(opts, out).await,
    }

    let mut client = connect(&socket_path(opts)?).await?;
    match &opts.cmd {
        Cmd::Watch => {
            let mut events = client.watch().await?;
            while let Some(event) = events.recv().await {
                render::event(out, &event?)?;
                out.flush()?;
            }
            return Ok(());
        }
        Cmd::Status { json } => return status(&mut client, *json, out).await,
        Cmd::Create { .. }
        | Cmd::Invite { .. }
        | Cmd::Join { .. }
        | Cmd::Leave { .. }
        | Cmd::Remove { .. }
        | Cmd::Peers { .. }
        | Cmd::Dns(_)
        | Cmd::Exit(_)
        | Cmd::Relay(_)
        | Cmd::Server { .. }
        | Cmd::Wg(_)
        | Cmd::Config(_)
        | Cmd::Doctor
        | Cmd::Up
        | Cmd::Down
        | Cmd::Daemon { .. } => {}
    }

    let names = names_for(&opts.cmd, &mut client).await?;
    let request = to_request(&opts.cmd, &names)?;
    let response = client.request(request).await?;
    render::response(out, &opts.cmd, response)
}

/// The names `cmd` needs resolved: networks always, and peers for exit and relay commands.
async fn names_for(cmd: &Cmd, client: &mut IpcClient) -> Result<Names, CliError> {
    let status = fetch_status(client).await?;
    let mut names = Names {
        networks: status
            .networks
            .iter()
            .map(|n| (n.id, n.name.clone()))
            .collect(),
        ..Names::default()
    };
    if let Cmd::Create {
        device_name: None, ..
    }
    | Cmd::Join {
        device_name: None, ..
    } = cmd
    {
        names.hostname = hostname();
    }
    if let Cmd::Exit(
        ExitCmd::Use { network, .. }
        | ExitCmd::Allow { network, .. }
        | ExitCmd::Deny { network, .. },
    )
    | Cmd::Remove { network, .. }
    | Cmd::Relay(RelayCmd::Advertise {
        network,
        node: Some(_),
        ..
    }) = cmd
    {
        let network = names.network(network)?;
        names.peers = fetch_peers(client, network)
            .await?
            .into_iter()
            .map(|p| (p.node, p.name, p.display_name))
            .collect();
    }
    Ok(names)
}

async fn status(client: &mut IpcClient, json: bool, out: &mut impl Write) -> Result<(), CliError> {
    let status = fetch_status(client).await?;
    let mut peers = Vec::with_capacity(status.networks.len());
    for network in &status.networks {
        peers.push(fetch_peers(client, network.id).await?);
    }
    if json {
        render::status_json(out, &status, &peers)?;
    } else {
        render::status(out, &status, &peers)?;
    }
    Ok(())
}

async fn doctor(opts: &Opts, out: &mut impl Write) -> Result<(), CliError> {
    let socket = socket_path(opts)?;
    let findings = match IpcClient::connect(&socket).await {
        Ok(mut client) => {
            let status = fetch_status(&mut client).await?;
            let mut peers = Vec::with_capacity(status.networks.len());
            for network in &status.networks {
                peers.push(fetch_peers(&mut client, network.id).await?);
            }
            doctor::diagnose(&status, &peers)
        }
        Err(IpcError::Io(error)) => {
            let port = load_config(None)?.network.listen_port;
            doctor::offline(&socket, &error, port)
        }
        Err(error) => return Err(error.into()),
    };
    doctor::render(out, &findings)?;
    let failed = findings
        .iter()
        .filter(|f| f.level == doctor::Level::Fail)
        .count();
    if failed > 0 {
        return Err(CliError(format!("{failed} problem(s) need fixing")));
    }
    Ok(())
}

async fn fetch_status(client: &mut IpcClient) -> Result<kith_ipc::Status, CliError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any other answer is a protocol error"
    )]
    match client.request(Request::Status).await? {
        Response::Status(status) => Ok(status),
        _other => Err(IpcError::Unexpected.into()),
    }
}

async fn fetch_peers(
    client: &mut IpcClient,
    network: NetworkId,
) -> Result<Vec<kith_ipc::PeerInfo>, CliError> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any other answer is a protocol error"
    )]
    match client.request(Request::Peers { network }).await? {
        Response::Peers(peers) => Ok(peers),
        _other => Err(IpcError::Unexpected.into()),
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().split('.').next().unwrap_or_default().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".to_owned())
}
