//! Human and JSON output. JSON is hand-written to avoid a serializer dependency.

use std::io::{self, Write};

use kith_ipc::{Event, IpcError, PathKind, PeerInfo, Response, Status};

use crate::cli::{Cmd, DnsCmd, ExitCmd, RelayCmd, WgCmd};
use crate::exec::CliError;

/// The PATH column: offline comes from liveness, never from a missing path.
fn path_cell(peer: &PeerInfo) -> &'static str {
    match (peer.online, peer.path) {
        (false, _) => "offline",
        (true, None) => "-",
        (true, path) => path_name(path),
    }
}

fn path_name(path: Option<PathKind>) -> &'static str {
    match path {
        Some(PathKind::Direct) => "direct",
        Some(PathKind::Relay) => "relay",
        Some(PathKind::Stealth) => "stealth",
        Some(_) => "other",
        None => "offline",
    }
}

fn handshake_age(secs: Option<u64>) -> String {
    let Some(secs) = secs else {
        return "never".to_owned();
    };
    let (value, unit) = match secs {
        0..60 => (secs, "s"),
        60..3600 => (secs / 60, "m"),
        3600..86_400 => (secs / 3600, "h"),
        _ => (secs / 86_400, "d"),
    };
    format!("{value}{unit} ago")
}

/// Binary units with one truncated decimal, like `1.1 MiB`.
fn byte_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut scale: u64 = 1024;
    let mut tenths = None;
    for unit in UNITS {
        if bytes < scale {
            break;
        }
        tenths = Some((u128::from(bytes) * 10 / u128::from(scale), unit));
        scale = scale.saturating_mul(1024);
    }
    match tenths {
        Some((tenths, unit)) => format!("{}.{} {unit}", tenths / 10, tenths % 10),
        None => format!("{bytes} B"),
    }
}

fn write_row<S: AsRef<str>>(
    out: &mut impl Write,
    indent: &str,
    cells: &[S],
    widths: &[usize],
) -> io::Result<()> {
    out.write_all(indent.as_bytes())?;
    let last = cells.len().saturating_sub(1);
    for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if i == last {
            writeln!(out, "{}", cell.as_ref())?;
        } else {
            write!(out, "{:width$}  ", cell.as_ref())?;
        }
    }
    Ok(())
}

fn peer_table(out: &mut impl Write, peers: &[PeerInfo], indent: &str) -> io::Result<()> {
    const HEADER: [&str; 8] = [
        "NAME",
        "ADDRESS",
        "PATH",
        "ENDPOINT",
        "HANDSHAKE",
        "RX",
        "TX",
        "NODE",
    ];
    if peers.is_empty() {
        return writeln!(out, "{indent}no peers yet");
    }
    let rows: Vec<[String; 8]> = peers
        .iter()
        .map(|peer| {
            [
                peer.name.clone(),
                peer.address.clone(),
                path_cell(peer).to_owned(),
                peer.endpoint.clone().unwrap_or_else(|| "-".to_owned()),
                handshake_age(peer.last_handshake_secs),
                byte_size(peer.rx_bytes),
                byte_size(peer.tx_bytes),
                peer.node.short().to_string(),
            ]
        })
        .collect();
    let mut widths = HEADER.map(str::len);
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    write_row(out, indent, &HEADER, &widths)?;
    for row in &rows {
        write_row(out, indent, row, &widths)?;
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
    if let Some(exit) = &status.exit_node {
        writeln!(out, "internet traffic goes through {exit}")?;
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
    notes(out, status)
}

/// Conflicts, drops and recent warnings, when there are any.
fn notes(out: &mut impl Write, status: &Status) -> io::Result<()> {
    let drops = status
        .drops
        .iter()
        .map(|(reason, count)| format!("{count} packets dropped: {reason}"));
    let lines: Vec<String> = status
        .route_conflicts
        .iter()
        .map(|c| format!("{c} overlaps kith's addresses (another VPN?)"))
        .chain(drops)
        .chain(
            status
                .warnings
                .iter()
                .map(|w| format!("{}s ago: {}", w.secs_ago, w.message)),
        )
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    writeln!(out, "\nnotes (run `kith doctor` for fixes):")?;
    for line in lines {
        writeln!(out, "  {line}")?;
    }
    Ok(())
}

fn json_strings(out: &mut impl Write, items: impl IntoIterator<Item = String>) -> io::Result<()> {
    out.write_all(b"[")?;
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        json_str(out, &item)?;
    }
    out.write_all(b"]")
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
            Some(_) => write!(out, ",\"path\":\"{}\"", path_name(peer.path))?,
            None => out.write_all(b",\"path\":null")?,
        }
        out.write_all(b",\"path_reason\":")?;
        match &peer.path_reason {
            Some(reason) => json_str(out, reason)?,
            None => out.write_all(b"null")?,
        }
        out.write_all(b",\"endpoint\":")?;
        match &peer.endpoint {
            Some(endpoint) => json_str(out, endpoint)?,
            None => out.write_all(b"null")?,
        }
        match peer.last_handshake_secs {
            Some(secs) => write!(out, ",\"last_handshake_secs\":{secs}")?,
            None => out.write_all(b",\"last_handshake_secs\":null")?,
        }
        write!(
            out,
            ",\"rx_bytes\":{},\"tx_bytes\":{},\"online\":{},\"display_name\":",
            peer.rx_bytes, peer.tx_bytes, peer.online
        )?;
        json_str(out, &peer.display_name)?;
        out.write_all(b"}")?;
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
        ",\"exit\":{},\"relay\":{},\"listen_port\":{},\"exit_node\":",
        status.exit, status.relay, status.listen_port
    )?;
    match &status.exit_node {
        Some(exit) => json_str(out, exit)?,
        None => out.write_all(b"null")?,
    }
    out.write_all(b",\"dns_upstreams\":")?;
    json_strings(out, status.dns_upstreams.iter().cloned())?;
    out.write_all(b",\"route_conflicts\":")?;
    json_strings(out, status.route_conflicts.iter().cloned())?;
    out.write_all(b",\"drops\":{")?;
    for (i, (reason, count)) in status.drops.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        json_str(out, reason)?;
        write!(out, ":{count}")?;
    }
    out.write_all(b"},\"warnings\":[")?;
    for (i, warning) in status.warnings.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, "{{\"secs_ago\":{},\"message\":", warning.secs_ago)?;
        json_str(out, &warning.message)?;
        out.write_all(b"}")?;
    }
    out.write_all(b"],\"networks\":[")?;
    for (i, (network, peers)) in status.networks.iter().zip(peers).enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, "{{\"id\":\"{}\",\"name\":", network.id)?;
        json_str(out, &network.name)?;
        write!(
            out,
            ",\"peers_online\":{},\"peers_total\":{},\"relays\":{},\"peers\":",
            network.peers_online, network.peers_total, network.relays
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
        (Cmd::Join { device_name, .. }, Response::Joined { network, device }) => {
            writeln!(out, "joined network {} as {device}", network.short())?;
            if let Some(asked) = device_name.as_deref().filter(|asked| *asked != device) {
                writeln!(out, "(another device already has the name `{asked}`)")?;
            }
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
        (Cmd::Remove { device, network }, Response::DeviceRemoved) => {
            writeln!(out, "removed {device} from {network}")?;
        }
        (Cmd::Exit(ExitCmd::Allow { node, .. }), Response::ExitGrantSet) => {
            writeln!(out, "{node} may now act as an exit node")?;
        }
        (Cmd::Exit(ExitCmd::Deny { node, .. }), Response::ExitGrantSet) => {
            writeln!(out, "{node} may no longer act as an exit node")?;
        }
        (Cmd::Relay(RelayCmd::Advertise { name, .. }), Response::RelayAdvertised { address }) => {
            writeln!(out, "advertised relay {name} at {address}")?;
        }
        (Cmd::Relay(RelayCmd::Remove { name, .. }), Response::RelayRemoved) => {
            writeln!(out, "removed relay {name}")?;
        }
        (Cmd::Server { exit, relay }, Response::ServerModeSet { notes }) => {
            writeln!(
                out,
                "exit node {}, relay {}",
                if *exit { "on" } else { "off" },
                if *relay { "on" } else { "off" }
            )?;
            for note in notes {
                writeln!(out, "{note}")?;
            }
        }
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
