//! Human and JSON output. JSON is hand-written to avoid a serializer dependency.

use std::io::{self, Write};

use kith_ipc::{Event, IpcError, PathKind, PeerInfo, Response, Status};

use crate::cli::{Cmd, DnsCmd, ExitCmd, WgCmd};
use crate::exec::CliError;

fn path_name(path: Option<PathKind>) -> &'static str {
    match path {
        Some(PathKind::Direct) => "direct",
        Some(PathKind::Relay) => "relay",
        Some(PathKind::Stealth) => "stealth",
        Some(_) => "other",
        None => "offline",
    }
}

fn peer_table(out: &mut impl Write, peers: &[PeerInfo], indent: &str) -> io::Result<()> {
    if peers.is_empty() {
        return writeln!(out, "{indent}no peers yet");
    }
    let name_w = peers.iter().map(|p| p.name.len()).max().unwrap_or(0).max(4);
    let addr_w = peers
        .iter()
        .map(|p| p.address.len())
        .max()
        .unwrap_or(0)
        .max(7);
    writeln!(
        out,
        "{indent}{:name_w$}  {:addr_w$}  {:7}  NODE",
        "NAME", "ADDRESS", "PATH"
    )?;
    for peer in peers {
        writeln!(
            out,
            "{indent}{:name_w$}  {:addr_w$}  {:7}  {}",
            peer.name,
            peer.address,
            path_name(peer.path),
            peer.node.short()
        )?;
    }
    Ok(())
}

/// `kith status`, for people. `peers` runs parallel to `status.networks`.
pub(crate) fn status(
    out: &mut impl Write,
    status: &Status,
    peers: &[Vec<PeerInfo>],
) -> io::Result<()> {
    match status.node {
        Some(node) => write!(out, "device {}", node.short())?,
        None => write!(out, "device not set up yet")?,
    }
    let roles: Vec<&str> = [(status.exit, "exit node"), (status.relay, "relay")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    if roles.is_empty() {
        writeln!(out)?;
    } else {
        writeln!(out, " ({})", roles.join(", "))?;
    }
    if status.networks.is_empty() {
        return writeln!(
            out,
            "no networks yet: `kith create <name>` or `kith join <link>`"
        );
    }
    for (network, peers) in status.networks.iter().zip(peers) {
        writeln!(
            out,
            "\nnetwork {} ({})  {}/{} online",
            network.name,
            network.id.short(),
            network.peers_online,
            network.peers_total
        )?;
        peer_table(out, peers, "  ")?;
    }
    Ok(())
}

fn json_str(out: &mut impl Write, text: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    for ch in text.chars() {
        match ch {
            '"' => out.write_all(b"\\\"")?,
            '\\' => out.write_all(b"\\\\")?,
            '\n' => out.write_all(b"\\n")?,
            c if u32::from(c) < 0x20 => write!(out, "\\u{:04x}", u32::from(c))?,
            c => write!(out, "{c}")?,
        }
    }
    out.write_all(b"\"")
}

fn peers_json(out: &mut impl Write, peers: &[PeerInfo]) -> io::Result<()> {
    out.write_all(b"[")?;
    for (i, peer) in peers.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, "{{\"node\":\"{}\",\"name\":", peer.node)?;
        json_str(out, &peer.name)?;
        out.write_all(b",\"address\":")?;
        json_str(out, &peer.address)?;
        match peer.path {
            Some(_) => write!(out, ",\"path\":\"{}\"}}", path_name(peer.path))?,
            None => out.write_all(b",\"path\":null}")?,
        }
    }
    out.write_all(b"]")
}

/// `kith status --json`.
pub(crate) fn status_json(
    out: &mut impl Write,
    status: &Status,
    peers: &[Vec<PeerInfo>],
) -> io::Result<()> {
    match status.node {
        Some(node) => write!(out, "{{\"node\":\"{node}\"")?,
        None => out.write_all(b"{\"node\":null")?,
    }
    write!(
        out,
        ",\"exit\":{},\"relay\":{},\"networks\":[",
        status.exit, status.relay
    )?;
    for (i, (network, peers)) in status.networks.iter().zip(peers).enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, "{{\"id\":\"{}\",\"name\":", network.id)?;
        json_str(out, &network.name)?;
        write!(
            out,
            ",\"peers_online\":{},\"peers_total\":{},\"peers\":",
            network.peers_online, network.peers_total
        )?;
        peers_json(out, peers)?;
        out.write_all(b"}")?;
    }
    out.write_all(b"]}\n")
}

/// One line per event for `kith watch`.
pub(crate) fn event(out: &mut impl Write, event: &Event) -> io::Result<()> {
    match event {
        Event::PeerConnected {
            network,
            node,
            path,
        } => writeln!(
            out,
            "peer {} connected in {} ({})",
            node.short(),
            network.short(),
            path_name(Some(*path))
        ),
        Event::PeerDisconnected { network, node } => {
            writeln!(
                out,
                "peer {} disconnected from {}",
                node.short(),
                network.short()
            )
        }
        Event::NetworkChanged { network } => writeln!(out, "network {} changed", network.short()),
        Event::Warning { message } => writeln!(out, "warning: {message}"),
        _ => writeln!(out, "unknown event"),
    }
}

/// The answer to any single-exchange command.
pub(crate) fn response(
    out: &mut impl Write,
    cmd: &Cmd,
    response: Response,
) -> Result<(), CliError> {
    match (cmd, response) {
        (Cmd::Create { name, .. }, Response::NetworkCreated { network }) => writeln!(
            out,
            "created network {name} ({})\nadd a device: kith invite {name}",
            network.short()
        )?,
        (Cmd::Invite { .. }, Response::Invite { invite_link }) => writeln!(
            out,
            "{invite_link}\n\non the other device run: kith join {invite_link}"
        )?,
        (Cmd::Join { .. }, Response::Joined { network }) => {
            writeln!(out, "joined network {}", network.short())?;
        }
        (Cmd::Leave { network }, Response::Left) => writeln!(out, "left {network}")?,
        (Cmd::Peers { json: true, .. }, Response::Peers(peers)) => {
            peers_json(out, &peers)?;
            writeln!(out)?;
        }
        (Cmd::Peers { .. }, Response::Peers(peers)) => peer_table(out, &peers, "")?,
        (Cmd::Dns(DnsCmd::Set { name, .. }), Response::RecordSet) => writeln!(out, "set {name}")?,
        (Cmd::Dns(DnsCmd::Rm { name, .. }), Response::RecordDeleted) => {
            writeln!(out, "removed {name}")?;
        }
        (Cmd::Exit(ExitCmd::Use { node, .. }), Response::ExitSet) => {
            writeln!(out, "internet traffic now goes through {node}")?;
        }
        (Cmd::Exit(ExitCmd::Off { .. }), Response::ExitSet) => writeln!(out, "exit node off")?,
        (Cmd::Server { exit, relay }, Response::ServerModeSet) => writeln!(
            out,
            "exit node {}, relay {}",
            if *exit { "on" } else { "off" },
            if *relay { "on" } else { "off" }
        )?,
        (Cmd::Wg(WgCmd::Import { name, .. }), Response::Imported { network }) => {
            writeln!(out, "imported {name} ({})", network.short())?;
        }
        (Cmd::Wg(WgCmd::Export { .. }), Response::WireGuardConfig { conf_text }) => {
            out.write_all(conf_text.as_bytes())?;
        }
        (_, other) => {
            drop(other);
            return Err(IpcError::Unexpected.into());
        }
    }
    Ok(())
}
