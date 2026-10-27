//! `kith doctor`: reads the daemon's state and says what is wrong and how to fix it.

use std::io::{self, Write};
use std::path::Path;

use kith_ipc::{PathKind, PeerInfo, Status};

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Ok,
    Warn,
    Fail,
}

/// One check's result, with a fix when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    pub level: Level,
    pub what: String,
    pub fix: Option<String>,
}

fn finding(level: Level, what: String, fix: Option<&str>) -> Finding {
    Finding {
        level,
        what,
        fix: fix.map(str::to_owned),
    }
}

/// Checks a running daemon. `peers` runs parallel to `status.networks`.
pub(crate) fn diagnose(status: &Status, peers: &[Vec<PeerInfo>]) -> Vec<Finding> {
    let mut found = vec![finding(
        Level::Ok,
        format!("daemon is running on port {}", status.listen_port),
        None,
    )];
    if status.networks.is_empty() {
        found.push(finding(
            Level::Warn,
            "this device is not in any network".to_owned(),
            Some("kith create <name>, or kith join <link>"),
        ));
    }
    for (network, peers) in status.networks.iter().zip(peers) {
        if peers.is_empty() {
            found.push(finding(
                Level::Ok,
                format!("{}: no other devices yet", network.name),
                Some("add one with `kith invite`"),
            ));
        }
        for peer in peers {
            found.push(check_peer(&network.name, peer));
        }
    }
    if let Some(exit) = &status.exit_node {
        let reachable = status
            .networks
            .iter()
            .zip(peers)
            .flat_map(|(network, peers)| peers.iter().map(move |p| (network, p)))
            .any(|(network, peer)| {
                *exit == format!("{}/{}", network.name, peer.name) && online(peer)
            });
        found.push(if reachable {
            finding(Level::Ok, format!("exit node {exit} is reachable"), None)
        } else {
            finding(
                Level::Fail,
                format!("exit node {exit} is unreachable, so internet traffic has nowhere to go"),
                Some("kith exit off, or bring the exit node back"),
            )
        });
    }
    for conflict in &status.route_conflicts {
        found.push(finding(
            Level::Warn,
            format!("{conflict} overlaps kith's addresses; kith peers there are unreachable"),
            Some("quit the other VPN (often Tailscale) while using kith"),
        ));
    }
    if !status.networks.is_empty() && status.dns_upstreams.is_empty() {
        found.push(finding(
            Level::Warn,
            "no upstream DNS servers: names outside the mesh will not resolve".to_owned(),
            Some("set dns.fallback_upstreams in the config"),
        ));
    }
    for (reason, count) in &status.drops {
        let (what, fix) = match reason.as_str() {
            "policy" => (
                "refused by the network's policy",
                Some("check the policy an admin set for this network"),
            ),
            "no_endpoint" | "no_route" => ("had no path to a peer", None),
            "source_not_allowed" => ("came from an address the sender does not own", None),
            _ => ("were dropped", None),
        };
        found.push(finding(
            Level::Warn,
            format!("{count} packets {what} ({reason})"),
            fix,
        ));
    }
    for warning in &status.warnings {
        found.push(finding(
            Level::Warn,
            format!("{}s ago: {}", warning.secs_ago, warning.message),
            None,
        ));
    }
    found
}

fn online(peer: &PeerInfo) -> bool {
    peer.path.is_some() && peer.last_handshake_secs.is_some_and(|secs| secs < 180)
}

fn check_peer(network: &str, peer: &PeerInfo) -> Finding {
    let reason = peer.path_reason.as_deref().unwrap_or("no path information");
    let name = format!("{network}/{}", peer.name);
    match peer.path {
        _ if !online(peer) => finding(
            Level::Warn,
            format!("{name} is unreachable: {reason}"),
            Some("make sure it is on, or run a relay with `kith server --relay` on a public host"),
        ),
        Some(PathKind::Direct) => finding(Level::Ok, format!("{name}: {reason}"), None),
        _ => finding(
            Level::Warn,
            format!("{name} works but is slower: {reason}"),
            Some("open UDP port 41641 on either side's firewall for a direct path"),
        ),
    }
}

/// Explains why the daemon at `socket` cannot be reached.
pub(crate) fn offline(socket: &Path, error: &io::Error, port: u16) -> Vec<Finding> {
    let mut found = Vec::new();
    if error.kind() == io::ErrorKind::PermissionDenied {
        found.push(finding(
            Level::Fail,
            format!("no permission to use {}", socket.display()),
            Some("run with sudo, or join the kith group"),
        ));
        return found;
    }
    found.push(finding(
        Level::Fail,
        format!("the daemon is not running ({} {error})", socket.display()),
        Some("sudo kith daemon, or start it with your service manager"),
    ));
    if port_taken(port) {
        found.push(finding(
            Level::Fail,
            format!("port {port} is held by another program, so the daemon could not start"),
            Some("quit the other VPN (Tailscale uses 41641 too), or set network.listen_port"),
        ));
    }
    found
}

fn port_taken(port: u16) -> bool {
    std::net::UdpSocket::bind(("0.0.0.0", port)).is_err()
        || std::net::TcpListener::bind(("0.0.0.0", port)).is_err()
}

/// Prints findings, one per line, with fixes indented below.
pub(crate) fn render(out: &mut impl Write, findings: &[Finding]) -> io::Result<()> {
    for found in findings {
        let tag = match found.level {
            Level::Ok => "ok  ",
            Level::Warn => "warn",
            Level::Fail => "FAIL",
        };
        writeln!(out, "{tag}  {}", found.what)?;
        if let Some(fix) = &found.fix {
            writeln!(out, "      fix: {fix}")?;
        }
    }
    Ok(())
}
