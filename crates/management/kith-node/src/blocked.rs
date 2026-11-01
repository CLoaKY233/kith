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

/// One connection between two devices, whichever of their addresses it used.
type Key = (NetworkId, NodeId, NodeId, u8, Option<u16>);
/// One connection as the dataplane keeps it, by address.
type Slot = (NetworkId, IpAddr, IpAddr, u8, Option<u16>);

/// What the daemon has seen of refused connections.
#[derive(Debug, Default)]
pub(crate) struct BlockedLog {
    seen: BTreeMap<Key, Seen>,
    /// The dataplane's count for each connection it kept at the last look, so only what
    /// it sampled since is new. Never larger than the dataplane's own table.
    sampled: BTreeMap<Slot, u64>,
}

#[derive(Debug)]
struct Seen {
    info: BlockedInfo,
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
        let slots = samples.iter().map(|sample| {
            let slot = (
                sample.network,
                sample.source,
                sample.destination,
                sample.protocol,
                sample.port,
            );
            (slot, sample.count)
        });
        let device = |network: NetworkId, address: IpAddr| {
            maps.get(&network)
                .and_then(|map| device_at(map, local, address))
        };
        self.absorb(slots, device, |kind| journal.push(kind), (now, at_ms));
    }

    /// [`observe`](Self::observe) with the samples as `(slot, count)`, addresses resolved
    /// by `device`, and journal entries handed to `report`.
    ///
    /// Only connections with new refusals are resolved to devices, so a quiet tick costs
    /// one map lookup per sample.
    fn absorb(
        &mut self,
        slots: impl Iterator<Item = (Slot, u64)>,
        device: impl Fn(NetworkId, IpAddr) -> Option<(NodeId, String)>,
        mut report: impl FnMut(EntryKind),
        (now, at_ms): (Instant, u64),
    ) {
        let previous = core::mem::take(&mut self.sampled);
        let mut new: BTreeMap<Key, (u64, String, String)> = BTreeMap::new();
        for (slot, count) in slots {
            let before = previous.get(&slot).copied().unwrap_or(0);
            // A count that went down is a connection the dataplane dropped and saw again.
            let added = count.checked_sub(before).unwrap_or(count);
            self.sampled.insert(slot, count);
            if added == 0 {
                continue;
            }
            let (network, source, destination, protocol, port) = slot;
            let (Some((from, from_name)), Some((to, to_name))) =
                (device(network, source), device(network, destination))
            else {
                continue;
            };
            let (total, ..) = new
                .entry((network, from, to, protocol, port))
                .or_insert((0, from_name, to_name));
            *total = total.saturating_add(added);
        }
        for (key, (added, from_name, to_name)) in new {
            let seen = self
                .seen
                .entry(key)
                .or_insert_with(|| Seen::new(key, from_name, to_name));
            seen.unreported = seen.unreported.saturating_add(added);
            seen.info.count = seen.info.count.saturating_add(added);
            seen.info.last_ms = at_ms;
        }
        for seen in self.seen.values_mut() {
            let due = seen
                .reported_at
                .is_none_or(|at| now.saturating_duration_since(at) >= JOURNAL_EVERY);
            if seen.unreported > 0 && due {
                let info = &seen.info;
                report(EntryKind::Blocked {
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
            // Connections a rule since allowed go first, then the least recently refused.
            let oldest = self
                .seen
                .iter()
                .min_by_key(|(_, seen)| (seen.info.count > 0, seen.info.last_ms))
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
        let Some((from, to, target)) = devices(rule) else {
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
            if (*id, *source, *destination) == (network, from, to) && covered {
                seen.info.count = 0;
                seen.unreported = 0;
            }
        }
    }
}

impl Seen {
    fn new((network, from, to, protocol, port): Key, from_name: String, to_name: String) -> Self {
        let protocol = protocol_name(protocol);
        let rule = allow_rule(from, to, &protocol, port)
            .ok()
            .map(|rule| render(&rule))
            .unwrap_or_default();
        Self {
            info: BlockedInfo {
                network,
                from,
                from_name,
                to,
                to_name,
                protocol,
                port,
                count: 0,
                last_ms: 0,
                rule,
            },
            unreported: 0,
            reported_at: None,
        }
    }
}

/// The device a rule from [`allow_rule`] lets in, the device it lets that one reach, and
/// how; `None` for any other shape of rule.
pub(crate) fn devices(rule: &Rule) -> Option<(NodeId, NodeId, &Target)> {
    let ([Selector::Device(from)], [target]) = (rule.sources(), rule.destinations()) else {
        return None;
    };
    let Selector::Device(to) = target.selector() else {
        return None;
    };
    Some((*from, *to, target))
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
/// and nothing more. TCP, UDP and ICMP may be named or numbered; any other protocol
/// number can only be allowed as `any`.
pub(crate) fn allow_rule(
    from: NodeId,
    to: NodeId,
    protocol: &str,
    port: Option<u16>,
) -> Result<Rule, NodeError> {
    let invalid = |message: &str| NodeError::InvalidArgument(message.to_owned());
    if from == to {
        return Err(invalid("a device always reaches itself"));
    }
    let protocol = protocol.trim().to_ascii_lowercase();
    let name = match protocol.parse::<u8>() {
        Ok(number) => protocol_name(number),
        Err(_) => protocol,
    };
    let (protocol, ports) = match (name.as_str(), port) {
        ("tcp", Some(port)) => (Protocol::Tcp, PortSet::single(port)),
        ("udp", Some(port)) => (Protocol::Udp, PortSet::single(port)),
        ("tcp" | "udp", None) => return Err(invalid("tcp and udp need a port")),
        ("icmp", None) => (Protocol::Icmp, PortSet::All),
        ("any", None) => (Protocol::Any, PortSet::All),
        (other, None) if other.parse::<u8>().is_ok() => (Protocol::Any, PortSet::All),
        _ => {
            return Err(invalid(
                "protocol must be tcp or udp with a port, or icmp, any or a number without one",
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

#[cfg(test)]
mod tests {
    use core::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    const NET: NetworkId = NetworkId::from_bytes([1; 32]);
    const ALPHA: NodeId = NodeId::from_bytes([2; 32]);
    const SERVER: NodeId = NodeId::from_bytes([3; 32]);
    const ALPHA_V4: IpAddr = IpAddr::V4(Ipv4Addr::new(100, 64, 0, 2));
    const ALPHA_V6: IpAddr = IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2));
    const SERVER_V4: IpAddr = IpAddr::V4(Ipv4Addr::new(100, 64, 0, 3));
    const SERVER_V6: IpAddr = IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 3));

    const NOTHING: [u64; 0] = [];

    fn device(_: NetworkId, address: IpAddr) -> Option<(NodeId, String)> {
        if [ALPHA_V4, ALPHA_V6].contains(&address) {
            Some((ALPHA, "alpha".to_owned()))
        } else if [SERVER_V4, SERVER_V6].contains(&address) {
            Some((SERVER, "server".to_owned()))
        } else {
            None
        }
    }

    fn tick(
        log: &mut BlockedLog,
        slots: &[(IpAddr, IpAddr, u8, Option<u16>, u64)],
        (now, at_ms): (Instant, u64),
    ) -> Vec<u64> {
        let mut counts = Vec::new();
        let slots = slots
            .iter()
            .map(|&(source, destination, protocol, port, count)| {
                ((NET, source, destination, protocol, port), count)
            });
        log.absorb(
            slots,
            device,
            |kind| {
                if let EntryKind::Blocked { count, .. } = kind {
                    counts.push(count);
                }
            },
            (now, at_ms),
        );
        counts
    }

    fn count(log: &BlockedLog, port: u16) -> Option<u64> {
        log.list(NET)
            .iter()
            .find(|info| info.port == Some(port))
            .map(|info| info.count)
    }

    #[test]
    fn only_new_refusals_count_and_the_journal_hears_of_them_at_most_every_30_seconds() {
        let mut log = BlockedLog::default();
        let start = Instant::now();
        let ssh_v4 = (ALPHA_V4, SERVER_V4, 6, Some(22));
        let ssh_v6 = (ALPHA_V6, SERVER_V6, 6, Some(22));
        let with = |(s, d, p, port): (IpAddr, IpAddr, u8, Option<u16>), n| (s, d, p, port, n);

        assert_eq!(
            tick(&mut log, &[with(ssh_v4, 5), with(ssh_v6, 3)], (start, 0)),
            [8]
        );
        assert_eq!(
            count(&log, 22),
            Some(8),
            "both addresses are one connection"
        );

        let later = start + Duration::from_secs(2);
        assert_eq!(
            tick(&mut log, &[with(ssh_v4, 5), with(ssh_v6, 3)], (later, 1)),
            NOTHING
        );
        assert_eq!(
            count(&log, 22),
            Some(8),
            "the same sample again is nothing new"
        );

        // The dataplane dropped the v6 slot; what it still holds for v4 is not new again.
        assert_eq!(tick(&mut log, &[with(ssh_v4, 6)], (later, 1)), NOTHING);
        assert_eq!(count(&log, 22), Some(9));

        // It saw v6 again, from one.
        let due = start + JOURNAL_EVERY;
        assert_eq!(
            tick(&mut log, &[with(ssh_v4, 6), with(ssh_v6, 1)], (due, 2)),
            [2]
        );
        assert_eq!(count(&log, 22), Some(10));

        log.forget(NET, &allow_rule(ALPHA, SERVER, "tcp", Some(22)).unwrap());
        assert_eq!(count(&log, 22), None);
        assert_eq!(
            tick(&mut log, &[with(ssh_v4, 6), with(ssh_v6, 1)], (due, 2)),
            NOTHING
        );
        assert_eq!(
            count(&log, 22),
            None,
            "what was sampled before stays allowed"
        );
    }

    #[test]
    fn a_scan_keeps_the_log_bounded_and_a_rule_clears_only_what_it_allows() {
        let mut log = BlockedLog::default();
        let mut now = Instant::now();
        for round in 0..4u16 {
            now += JOURNAL_EVERY;
            let slots: Vec<_> = (0..256)
                .map(|i| (ALPHA_V4, SERVER_V4, 17, Some(round * 1000 + i), 1))
                .collect();
            tick(&mut log, &slots, (now, u64::from(round)));
            assert!(log.seen.len() <= KEEP);
            assert!(log.sampled.len() <= KEEP);
        }
        assert_eq!(
            count(&log, 3000),
            Some(1),
            "the latest connections are kept"
        );
        assert_eq!(count(&log, 0), None, "the oldest made room");

        log.forget(NET, &allow_rule(ALPHA, SERVER, "udp", Some(3001)).unwrap());
        assert_eq!((count(&log, 3000), count(&log, 3001)), (Some(1), None));
        log.forget(NET, &allow_rule(SERVER, ALPHA, "any", None).unwrap());
        assert_eq!(
            count(&log, 3000),
            Some(1),
            "the reverse direction allows nothing"
        );
    }

    #[test]
    fn the_allow_rule_opens_exactly_the_connection_asked_for() {
        let shape = |protocol: &str, port| {
            allow_rule(ALPHA, SERVER, protocol, port)
                .ok()
                .as_ref()
                .and_then(devices)
                .map(|(from, to, target)| {
                    assert_eq!((from, to), (ALPHA, SERVER));
                    (target.protocol(), target.ports().clone())
                })
        };
        assert_eq!(
            shape(" TCP ", Some(22)),
            Some((Protocol::Tcp, PortSet::single(22)))
        );
        assert_eq!(
            shape("6", Some(22)),
            Some((Protocol::Tcp, PortSet::single(22)))
        );
        assert_eq!(shape("6", None), None, "tcp by number still needs a port");
        assert_eq!(shape("58", None), Some((Protocol::Icmp, PortSet::All)));
        assert_eq!(shape("47", None), Some((Protocol::Any, PortSet::All)));
        assert_eq!(shape("icmp", Some(1)), None);
        assert_eq!(shape("47", Some(1)), None);
        assert_eq!(shape("300", None), None);
        assert!(allow_rule(ALPHA, ALPHA, "any", None).is_err());
    }
}
