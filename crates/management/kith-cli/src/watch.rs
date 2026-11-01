//! `kith watch`, `kith log` and `kith top`: what is happening, from the daemon's journal
//! and traffic history.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use kith_ipc::{Entry, EntryKind, PathKind, Request, Response, TrafficSeries};
use kith_types::NetworkId;

use crate::exec::{CliError, connect, fetch_peers, fetch_status};
use crate::render;

/// How often `kith top` redraws.
const TOP_REFRESH: Duration = Duration::from_secs(2);

/// Network names by id, to print `home/phone` rather than an id.
pub(crate) type NetworkNames = BTreeMap<NetworkId, String>;

/// Prints the state, then each change as it happens, until the daemon stops.
pub(crate) async fn watch(socket: &Path, out: &mut impl Write) -> Result<(), CliError> {
    // Follow the journal before reading the state, so a change between the two is shown
    // rather than lost; history is skipped, as the state already says it.
    let (next, mut entries) = connect(socket).await?.journal(Some(u64::MAX)).await?;
    let mut client = connect(socket).await?;
    let status = fetch_status(&mut client).await?;
    let mut peers = Vec::with_capacity(status.networks.len());
    for network in &status.networks {
        peers.push(fetch_peers(&mut client, network.id).await?);
    }
    render::status(out, &status, &peers)?;
    let names = status
        .networks
        .iter()
        .map(|network| (network.id, network.name.clone()))
        .collect();
    writeln!(out, "\nwatching for changes; ctrl-c to stop")?;
    out.flush()?;
    let mut expected = Some(next);
    while let Some(entry) = entries.recv().await {
        let entry = entry?;
        if entry.seq < next {
            continue;
        }
        note_missed(out, &mut expected, entry.seq)?;
        if matches!(entry.kind, EntryKind::Log { .. }) {
            continue;
        }
        writeln!(out, "{}", line(&entry, &names, offset(entry.at_ms)))?;
        out.flush()?;
    }
    writeln!(out, "kith stopped")?;
    Ok(())
}

/// Prints the journal: what happened and the daemon's log, then, with `follow`, each new
/// entry as it comes.
pub(crate) async fn log(
    socket: &Path,
    follow: bool,
    events_only: bool,
    out: &mut impl Write,
) -> Result<(), CliError> {
    let names = network_names(socket).await?;
    let (next, mut entries) = connect(socket).await?.journal(None).await?;
    if next == 0 && !follow {
        writeln!(out, "nothing has happened yet")?;
        return Ok(());
    }
    let mut expected = None;
    while let Some(entry) = entries.recv().await {
        let entry = entry?;
        note_missed(out, &mut expected, entry.seq)?;
        if !(events_only && matches!(entry.kind, EntryKind::Log { .. })) {
            writeln!(out, "{}", line(&entry, &names, offset(entry.at_ms)))?;
            out.flush()?;
        }
        if !follow && entry.seq.saturating_add(1) >= next {
            return Ok(());
        }
    }
    Ok(())
}

/// Redraws traffic per peer every two seconds, busiest first, until interrupted.
pub(crate) async fn top(socket: &Path, out: &mut impl Write) -> Result<(), CliError> {
    let names = network_names(socket).await?;
    let mut client = connect(socket).await?;
    let mut tick = tokio::time::interval(TOP_REFRESH);
    loop {
        tick.tick().await;
        let Response::Traffic(series) = client.request(Request::Traffic).await? else {
            return Err(CliError("the daemon sent something else".to_owned()));
        };
        // Clear the screen and go home, as `top` does.
        write!(out, "\x1b[H\x1b[2J")?;
        traffic_table(out, &series, &names)?;
        writeln!(out, "\nctrl-c to stop")?;
        out.flush()?;
    }
}

/// Says how many entries the daemon skipped before `seq` because this reader fell
/// behind, then expects the one after `seq`.
fn note_missed(
    out: &mut impl Write,
    expected: &mut Option<u64>,
    seq: u64,
) -> std::io::Result<()> {
    if let Some(missed) = expected.and_then(|want| seq.checked_sub(want)).filter(|n| *n > 0) {
        writeln!(out, "({missed} entries missed: this reader fell behind)")?;
    }
    *expected = Some(seq.saturating_add(1));
    Ok(())
}

async fn network_names(socket: &Path) -> Result<NetworkNames, CliError> {
    let status = fetch_status(&mut connect(socket).await?).await?;
    Ok(status
        .networks
        .into_iter()
        .map(|network| (network.id, network.name))
        .collect())
}

/// The table `kith top` draws: each peer's rate over the last two samples and its totals.
pub(crate) fn traffic_table(
    out: &mut impl Write,
    series: &[TrafficSeries],
    names: &NetworkNames,
) -> std::io::Result<()> {
    let mut rows: Vec<(u64, [String; 5])> = series
        .iter()
        .filter_map(|peer| {
            let last = peer.samples.last()?;
            let (down, up) = match peer
                .samples
                .len()
                .checked_sub(2)
                .and_then(|i| peer.samples.get(i))
            {
                Some(before) => {
                    let ms = last.at_ms.saturating_sub(before.at_ms).max(1);
                    let rate =
                        |now: u64, then: u64| now.saturating_sub(then).saturating_mul(1000) / ms;
                    (
                        rate(last.rx_bytes, before.rx_bytes),
                        rate(last.tx_bytes, before.tx_bytes),
                    )
                }
                None => (0, 0),
            };
            Some((
                down.saturating_add(up),
                [
                    printable(&format!(
                        "{}/{}",
                        network_name(names, peer.network),
                        peer.name
                    )),
                    format!("{}/s", render::byte_size(down)),
                    format!("{}/s", render::byte_size(up)),
                    render::byte_size(last.rx_bytes),
                    render::byte_size(last.tx_bytes),
                ],
            ))
        })
        .collect();
    if rows.is_empty() {
        return writeln!(out, "no peers yet");
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1[0].cmp(&b.1[0])));
    let header = ["PEER", "DOWN", "UP", "RECEIVED", "SENT"];
    let mut widths = header.map(str::len);
    for (_, row) in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    render::write_row(out, "", &header, &widths)?;
    for (_, row) in &rows {
        render::write_row(out, "", row, &widths)?;
    }
    Ok(())
}

/// One entry as a line: local time, then what happened, with control characters
/// escaped so text from peers or the log cannot drive the terminal or break the line.
pub(crate) fn line(entry: &Entry, names: &NetworkNames, utc_offset_secs: i64) -> String {
    format!(
        "{}  {}",
        clock(entry.at_ms, utc_offset_secs),
        printable(&describe(&entry.kind, names))
    )
}

/// `text` with every control character, such as an escape or a newline, written as an
/// escape sequence instead.
pub(crate) fn printable(text: &str) -> String {
    if !text.chars().any(char::is_control) {
        return text.to_owned();
    }
    text.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// What an entry says, in words.
pub(crate) fn describe(kind: &EntryKind, names: &NetworkNames) -> String {
    let who = |network: &NetworkId, name: &str| format!("{}/{name}", network_name(names, *network));
    match kind {
        EntryKind::PeerOnline {
            network,
            name,
            path,
            gateway,
            ..
        } => match (path, gateway) {
            (Some(path), _) => format!("{} online, {}", who(network, name), path_word(*path)),
            (None, Some(gateway)) => format!("{} online, via {gateway}", who(network, name)),
            (None, None) => format!("{} online", who(network, name)),
        },
        EntryKind::PeerOffline { network, name, .. } => format!("{} offline", who(network, name)),
        EntryKind::PathChanged {
            network, name, to, ..
        } => match to {
            Some(path) => format!("{} now {}", who(network, name), path_word(*path)),
            None => format!("{} path changed", who(network, name)),
        },
        EntryKind::EndpointChanged {
            network,
            name,
            endpoint,
            ..
        } => format!("{} moved to {endpoint}", who(network, name)),
        EntryKind::DeviceJoined {
            network,
            name,
            address,
            ..
        } => format!("{} joined ({address})", who(network, name)),
        EntryKind::DeviceRemoved { network, name, .. } => format!("{} removed", who(network, name)),
        EntryKind::ExitChanged { exit } => exit.as_ref().map_or_else(
            || "internet traffic no longer goes through an exit".to_owned(),
            |exit| format!("internet traffic now goes through {exit}"),
        ),
        EntryKind::Dropped { reason, count } => format!("{count} packets dropped: {reason}"),
        EntryKind::Blocked {
            network,
            from_name,
            to_name,
            protocol,
            port,
            count,
            ..
        } => {
            let what = port.map_or_else(|| protocol.clone(), |port| format!("{protocol} {port}"));
            format!(
                "policy blocked {} -> {to_name} {what} ({count} packets); see kith blocked",
                who(network, from_name)
            )
        }
        EntryKind::Warning { message } => format!("warning: {message}"),
        EntryKind::Log {
            level,
            target,
            message,
        } => format!("{level:<5} {target}: {message}"),
        EntryKind::JoinRequested { network, name, .. } => format!(
            "{} asks to join; kith approve or kith deny",
            who(network, name)
        ),
        _ => "something the daemon knows and this kith does not; update kith".to_owned(),
    }
}

fn network_name(names: &NetworkNames, network: NetworkId) -> String {
    names
        .get(&network)
        .cloned()
        .unwrap_or_else(|| network.short().to_string())
}

const fn path_word(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "direct",
        PathKind::Relay => "relayed",
        PathKind::Stealth => "stealth",
        _ => "other path",
    }
}

/// The local UTC offset at `at_ms`.
fn offset(at_ms: u64) -> i64 {
    kith_sys::utc_offset_secs(i64::try_from(at_ms / 1000).unwrap_or(0))
}

/// `at_ms` as `HH:MM:SS`, `utc_offset_secs` ahead of UTC.
pub(crate) fn clock(at_ms: u64, utc_offset_secs: i64) -> String {
    let secs = i64::try_from(at_ms / 1000)
        .unwrap_or(0)
        .saturating_add(utc_offset_secs)
        .rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_from_peers_cannot_drive_the_terminal() {
        let entry = Entry {
            seq: 0,
            at_ms: 0,
            kind: EntryKind::Warning {
                message: "\x1b]0;owned\x07two\nlines".into(),
            },
        };
        assert_eq!(
            line(&entry, &NetworkNames::new(), 0),
            "00:00:00  warning: \\u{1b}]0;owned\\u{7}two\\nlines"
        );
    }

    #[test]
    fn a_gap_in_sequence_numbers_is_reported_once() {
        let mut out = Vec::new();
        let mut expected = None;
        for seq in [5, 6, 9, 10] {
            note_missed(&mut out, &mut expected, seq).ok();
        }
        assert_eq!(
            String::from_utf8_lossy(&out),
            "(2 entries missed: this reader fell behind)\n"
        );
    }
}
