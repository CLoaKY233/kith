//! `kith doctor`: reads the daemon's state and says what is wrong and how to fix it.

use std::io::{self, Write};
use std::path::Path;

use kith_ipc::{NetworkSummary, PathKind, PeerInfo, Status};

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Ok,
    /// Worth knowing, but normal; no fix needed.
    Info,
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
    for network in &status.networks {
        for problem in &network.problems {
            found.push(Finding {
                level: Level::Warn,
                what: format!("{}: {}", network.name, problem.what),
                fix: Some(problem.fix.clone()),
            });
        }
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
            found.push(check_peer(network, peer, peers));
            if let Some(mtu) = peer.path_mtu.filter(|_| peer.online) {
                found.push(finding(
                    Level::Info,
                    format!(
                        "{}/{}: its path carries only {mtu}-byte packets; kith keeps TCP smaller, \
                         splits IPv4 and tells other senders to shrink",
                        network.name, peer.name
                    ),
                    None,
                ));
            }
        }
    }
    if let Some(exit) = &status.exit_node {
        let reachable = status
            .networks
            .iter()
            .zip(peers)
            .flat_map(|(network, peers)| peers.iter().map(move |p| (network, p)))
            .any(|(network, peer)| {
                *exit == format!("{}/{}", network.name, peer.name) && peer.online
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
        found.push(drop_finding(reason, *count));
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

/// What a drop counter means: most are normal traffic, a few point to a real problem.
fn drop_finding(reason: &str, count: u64) -> Finding {
    let (level, what, fix) = match reason {
        "no_route" | "no_endpoint" => (
            Level::Info,
            "were for a peer that is offline or an unused mesh address",
            None,
        ),
        "policy" => (
            Level::Info,
            "arrived unasked and the firewall refused them",
            None,
        ),
        "after_exit" => (
            Level::Info,
            "were late internet replies after the exit was turned off; their connections had ended",
            None,
        ),
        "too_big" => (
            Level::Info,
            "were bigger than their path carries; their senders were told the size that fits",
            None,
        ),
        "foreign_source" => (
            Level::Info,
            "left from an address that is not this device's, usually connections \
             opened before the exit was turned on; apps reconnect",
            None,
        ),
        "invalid" | "malformed" => (
            Level::Info,
            "were not kith traffic, as a public server always gets",
            None,
        ),
        "source_not_allowed" => (
            Level::Warn,
            "came from an address the sending peer does not own",
            Some("run `kith doctor` on that peer, and report it if it persists"),
        ),
        _ => (
            Level::Warn,
            "were dropped because a queue or table was full",
            Some("report this with `kith status --json`"),
        ),
    };
    finding(level, format!("{count} packets {what} ({reason})"), fix)
}

fn check_peer(network: &NetworkSummary, peer: &PeerInfo, peers: &[PeerInfo]) -> Finding {
    let reason = peer.path_reason.as_deref().unwrap_or("no path information");
    let name = format!("{}/{}", network.name, peer.name);
    let remove = format!(
        "if it is gone for good, `kith remove {} {}`",
        shell_word(&network.name),
        shell_word(&peer.name)
    );
    if !peer.online {
        let seen = match peer.last_seen_secs {
            Some(secs) => format!("last seen {} ago", ago(secs)),
            None => "never connected".to_owned(),
        };
        let fix = match &peer.gateway {
            Some(gateway) if peers.iter().any(|p| &p.name == gateway && !p.online) => format!(
                "it connects through {gateway}, which is offline too; start kith there first"
            ),
            Some(gateway) => format!(
                "turn its tunnel on in the WireGuard app and check it has internet; it connects through {gateway}; {remove}"
            ),
            None if network.relays > 0 => format!(
                "make sure kith is running on it and it has internet; it falls back to the relay when direct UDP is blocked; {remove}"
            ),
            None => format!(
                "make sure kith is running on it and it has internet; behind a strict firewall it also needs a relay (`kith server --relay` on a public host); {remove}"
            ),
        };
        return Finding {
            level: Level::Warn,
            what: format!("{name} is offline: {seen}"),
            fix: Some(fix),
        };
    }
    if let Some(gateway) = &peer.gateway {
        return finding(Level::Ok, format!("{name}: through {gateway}"), None);
    }
    match peer.path {
        Some(PathKind::Direct) => finding(Level::Ok, format!("{name}: {reason}"), None),
        None => finding(Level::Info, format!("{name} is connecting: {reason}"), None),
        Some(_) => finding(
            Level::Warn,
            format!("{name} works but is slower: {reason}"),
            Some("open UDP port 41641 on either side's firewall for a direct path"),
        ),
    }
}

/// `secs` as a rough age: seconds, minutes, hours or days.
fn ago(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h", secs / 3600),
        _ => format!("{} days", secs / 86_400),
    }
}

/// `word` as one shell argument: as it is when plain, else single-quoted.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Explains why the daemon at `socket` cannot be reached.
pub(crate) fn offline(socket: &Path, error: &io::Error, port: u16) -> Vec<Finding> {
    let mut found = Vec::new();
    if error.kind() == io::ErrorKind::PermissionDenied {
        found.push(finding(
            Level::Fail,
            format!("no permission to use {}", socket.display()),
            Some("run with sudo, or join the operator group (admin on macOS, kith on Linux)"),
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
            Level::Info => "info",
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
