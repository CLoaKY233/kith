//! Connections a network's policy refused: journal entries for them, the list behind
//! `kith blocked`, and the one rule that would allow each.

use core::net::IpAddr;
use core::time::Duration;
use std::collections::BTreeMap;

use kith_control::{NetMap, Networks};
use kith_dataplane::Blocked;
use kith_ipc::{BlockedInfo, EntryKind};
use kith_policy::{PortSet, Protocol, Rule, Selector, Target};
use kith_types::{NetworkId, NodeId};
use tokio::time::Instant;

use crate::journal::Journal;
use crate::types::NodeError;

/// The shortest time between two journal entries for the same connection.
const JOURNAL_EVERY: Duration = Duration::from_secs(30);
/// Connections remembered; the least recently refused are forgotten first.
const KEEP: usize = 256;

type Key = (NetworkId, NodeId, NodeId, u8, Option<u16>);

/// What the daemon has seen of refused connections.
#[derive(Debug, Default)]
pub(crate) struct BlockedLog {
    seen: BTreeMap<Key, Seen>,
}

#[derive(Debug)]
struct Seen {
    info: BlockedInfo,
    /// The dataplane's count when last looked at.
    sampled: u64,
    /// Refused since the last journal entry.
    unreported: u64,
    reported_at: Option<Instant>,
}

impl BlockedLog {
    /// Takes the dataplane's latest sample of refused connections, and journals what is
    /// new, at most once per connection every [`JOURNAL_EVERY`].
    pub(crate) fn observe(
        &mut self,
        samples: &[Blocked],
        maps: &Networks,
        (local, journal): (NodeId, &Journal),
        (now, at_ms): (Instant, u64),
    ) {
        let mut totals: BTreeMap<Key, (u64, BlockedInfo)> = BTreeMap::new();
        for sample in samples {
            let Some(map) = maps.get(&sample.network) else {
                continue;
            };
            let (Some((from, from_name)), Some((to, to_name))) = (
                device_at(map, local, sample.source),
                device_at(map, local, sample.destination),
            ) else {
                continue;
            };
            let key = (sample.network, from, to, sample.protocol, sample.port);
            let (total, _) = totals.entry(key).or_insert_with(|| {
                let protocol = protocol_name(sample.protocol);
                let rule = allow_rule(from, to, &protocol, sample.port)
                    .ok()
                    .map(|rule| render(&rule))
                    .unwrap_or_default();
                (
                    0,
                    BlockedInfo {
                        network: sample.network,
                        from,
                        from_name,
                        to,
                        to_name,
                        protocol,
                        port: sample.port,
                        count: 0,
                        last_ms: at_ms,
                        rule,
                    },
                )
            });
            *total = total.saturating_add(sample.count);
        }
        for (key, (total, info)) in totals {
            let seen = self.seen.entry(key).or_insert_with(|| Seen {
                info,
                sampled: 0,
                unreported: 0,
                reported_at: None,
            });
            let new = if total >= seen.sampled {
                total - seen.sampled
            } else {
                total
            };
            seen.sampled = total;
            if new > 0 {
                seen.unreported = seen.unreported.saturating_add(new);
                seen.info.count = seen.info.count.saturating_add(new);
                seen.info.last_ms = at_ms;
            }
        }
        for seen in self.seen.values_mut() {
            let due = seen
                .reported_at
                .is_none_or(|at| now.saturating_duration_since(at) >= JOURNAL_EVERY);
            if seen.unreported > 0 && due {
                let info = &seen.info;
                journal.push(EntryKind::Blocked {
                    network: info.network,
                    from: info.from,
                    from_name: info.from_name.clone(),
                    to: info.to,
                    to_name: info.to_name.clone(),
                    protocol: info.protocol.clone(),
                    port: info.port,
                    count: seen.unreported,
                });
                seen.unreported = 0;
                seen.reported_at = Some(now);
            }
        }
        while self.seen.len() > KEEP {
            let oldest = self
                .seen
                .iter()
                .min_by_key(|(_, seen)| seen.info.last_ms)
                .map(|(key, _)| *key);
            match oldest {
                Some(key) => self.seen.remove(&key),
                None => break,
            };
        }
    }

    /// The connections refused in `network`, most recent first.
    pub(crate) fn list(&self, network: NetworkId) -> Vec<BlockedInfo> {
        let mut list: Vec<BlockedInfo> = self
            .seen
            .iter()
            .filter(|((id, ..), seen)| *id == network && seen.info.count > 0)
            .map(|(_, seen)| seen.info.clone())
            .collect();
        list.sort_by_key(|info| core::cmp::Reverse(info.last_ms));
        list
    }

    /// Clears the connections `rule` now allows, once it is in `network`'s policy; what
    /// the dataplane sampled before stays counted, so only new refusals show again.
    pub(crate) fn forget(&mut self, network: NetworkId, rule: &Rule) {
        let ([Selector::Device(from)], [target]) = (rule.sources(), rule.destinations()) else {
            return;
        };
        let Selector::Device(to) = target.selector() else {
            return;
        };
        for ((id, source, destination, protocol, port), seen) in &mut self.seen {
            let covered = match target.protocol() {
                Protocol::Any => true,
                Protocol::Icmp => matches!(protocol, 1 | 58),
                Protocol::Tcp => {
                    *protocol == 6 && port.is_some_and(|p| allows_port(target.ports(), p))
                }
                Protocol::Udp => {
                    *protocol == 17 && port.is_some_and(|p| allows_port(target.ports(), p))
                }
            };
            if (*id, *source, *destination) == (network, *from, *to) && covered {
                seen.info.count = 0;
                seen.unreported = 0;
            }
        }
    }
}

fn allows_port(ports: &PortSet, port: u16) -> bool {
    match ports {
        PortSet::All => true,
        PortSet::Only(ranges) => ranges.contains(port),
    }
}

/// The device holding `address` in `map`, with its name.
fn device_at(map: &NetMap, local: NodeId, address: IpAddr) -> Option<(NodeId, String)> {
    let holds = |peer: &kith_control::Peer| {
        address == IpAddr::V4(peer.addresses.v4) || address == IpAddr::V6(peer.addresses.v6)
    };
    map.peers()
        .iter()
        .chain(map.clients().iter().map(|(id, client)| (id, &client.peer)))
        .find(|(_, peer)| holds(peer))
        .map(|(id, peer)| (*id, peer.label.clone()))
        .or_else(|| {
            map.local_device()
                .filter(|local| holds(local))
                .map(|device| (local, device.label.clone()))
        })
}

fn protocol_name(number: u8) -> String {
    match number {
        6 => "tcp".to_owned(),
        17 => "udp".to_owned(),
        1 | 58 => "icmp".to_owned(),
        other => other.to_string(),
    }
}

/// The rule that lets `from` reach `to` with `protocol` (and `port`, for TCP and UDP),
/// and nothing more. A protocol given by number can only be allowed as `any`.
pub(crate) fn allow_rule(
    from: NodeId,
    to: NodeId,
    protocol: &str,
    port: Option<u16>,
) -> Result<Rule, NodeError> {
    let invalid = |message: &str| NodeError::InvalidArgument(message.to_owned());
    let (protocol, ports) = match protocol.trim().to_ascii_lowercase().as_str() {
        name @ ("tcp" | "udp") => {
            let port = port.ok_or_else(|| invalid("tcp and udp need a port"))?;
            let protocol = if name == "tcp" {
                Protocol::Tcp
            } else {
                Protocol::Udp
            };
            (protocol, PortSet::single(port))
        }
        "icmp" if port.is_none() => (Protocol::Icmp, PortSet::All),
        "any" if port.is_none() => (Protocol::Any, PortSet::All),
        other if other.parse::<u8>().is_ok() && port.is_none() => (Protocol::Any, PortSet::All),
        _ => {
            return Err(invalid(
                "protocol must be tcp or udp with a port, or icmp or any without one",
            ));
        }
    };
    let target = Target::new(Selector::Device(to), protocol, ports)
        .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
    Rule::new(vec![Selector::Device(from)], vec![target])
        .map_err(|error| NodeError::InvalidArgument(error.to_string()))
}

/// `rule` in the TOML syntax `kith policy` documents take.
fn render(rule: &Rule) -> String {
    kith_policy::Policy::new(vec![rule.clone()])
        .ok()
        .and_then(|policy| toml::to_string(&policy).ok())
        .unwrap_or_default()
}
