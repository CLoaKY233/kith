//! Link reports: each device gossips which peers it reaches and how, for the network map.
//!
//! A report replaces the reporter's previous one. Reports are sent when the set of links
//! or their paths change, and every [`REPORT_INTERVAL`] so latency and rates stay fresh.

use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet};

use kith_channel::{Link, LinkPath};
use kith_ipc::PathKind;
use kith_types::{NetworkId, NodeId};
use tokio::time::Instant;

/// How often a device repeats its report when nothing changed.
pub(crate) const REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// The most links one topology answer holds, so it stays well inside an IPC frame.
pub(crate) const MAX_RESPONSE_LINKS: usize = 1500;

/// How long a report is kept after it arrived: a device that stops reporting is gone.
const REPORT_EXPIRY: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Received {
    at: Instant,
    links: Vec<Link>,
}

/// The latest report of every member, and what this device last sent.
#[derive(Debug)]
pub(crate) struct Topology {
    local: NodeId,
    reports: BTreeMap<(NetworkId, NodeId), Received>,
    sent: BTreeMap<NetworkId, (Instant, Vec<Link>)>,
}

impl Topology {
    /// No reports yet, on the device `local`.
    pub(crate) const fn new(local: NodeId) -> Self {
        Self {
            local,
            reports: BTreeMap::new(),
            sent: BTreeMap::new(),
        }
    }

    /// Keeps `from`'s report for `network` in place of its last one. `members` are the
    /// network's other devices; a report from anyone else, or from this device, is dropped,
    /// as are links to a non-member and repeated links to one peer.
    pub(crate) fn receive(
        &mut self,
        network: NetworkId,
        from: NodeId,
        mut links: Vec<Link>,
        members: &BTreeSet<NodeId>,
        at: Instant,
    ) {
        let local = self.local;
        if from == local || !members.contains(&from) {
            return;
        }
        let mut seen = BTreeSet::new();
        links.retain(|link| {
            link.peer != from
                && (link.peer == local || members.contains(&link.peer))
                && seen.insert(link.peer)
        });
        self.reports.insert((network, from), Received { at, links });
    }

    /// Drops reports for networks this device left and from devices no longer members.
    pub(crate) fn retain(&mut self, members: &BTreeMap<NetworkId, BTreeSet<NodeId>>, at: Instant) {
        self.reports.retain(|(network, from), report| {
            at.saturating_duration_since(report.at) < REPORT_EXPIRY
                && members
                    .get(network)
                    .is_some_and(|nodes| nodes.contains(from))
        });
        self.sent.retain(|network, _| members.contains_key(network));
    }

    /// Whether `links` should be sent for `network` now: they differ in who is reached or
    /// how from the last report, or the last report is [`REPORT_INTERVAL`] old.
    pub(crate) fn due(&self, network: NetworkId, links: &[Link], at: Instant) -> bool {
        let shape = |links: &[Link]| -> Vec<(NodeId, LinkPath, Option<NodeId>)> {
            links
                .iter()
                .map(|link| (link.peer, link.path, link.relay))
                .collect()
        };
        self.sent.get(&network).is_none_or(|(when, before)| {
            at.saturating_duration_since(*when) >= REPORT_INTERVAL || shape(before) != shape(links)
        })
    }

    /// Records that `links` were sent for `network`.
    pub(crate) fn sent(&mut self, network: NetworkId, links: Vec<Link>, at: Instant) {
        self.sent.insert(network, (at, links));
    }

    /// Every other member's latest report for `network`, with its age.
    pub(crate) fn reports(
        &self,
        network: NetworkId,
        at: Instant,
    ) -> impl Iterator<Item = (NodeId, Duration, &[Link])> {
        self.reports
            .iter()
            .filter(move |((id, _), _)| *id == network)
            .map(move |((_, from), report)| {
                (
                    *from,
                    at.saturating_duration_since(report.at),
                    report.links.as_slice(),
                )
            })
    }
}

/// The wire form of a path.
pub(crate) const fn link_path(kind: PathKind) -> LinkPath {
    match kind {
        PathKind::Direct => LinkPath::Direct,
        PathKind::Stealth => LinkPath::Stealth,
        PathKind::Relay | _ => LinkPath::Relay,
    }
}

/// The IPC form of a path.
pub(crate) const fn path_kind(path: LinkPath) -> PathKind {
    match path {
        LinkPath::Direct => PathKind::Direct,
        LinkPath::Relay => PathKind::Relay,
        LinkPath::Stealth => PathKind::Stealth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(byte: u8) -> NodeId {
        NodeId::from_bytes([byte; 32])
    }

    fn link(peer: NodeId, path: LinkPath) -> Link {
        Link {
            peer,
            path,
            relay: None,
            rtt_ms: Some(5),
            bytes_per_sec: 100,
        }
    }

    #[test]
    fn reports_replace_each_other_and_only_members_count() {
        let network = NetworkId::from_bytes([9; 32]);
        let other = NetworkId::from_bytes([8; 32]);
        let (local, a, b, stranger) = (node(1), node(2), node(3), node(4));
        let members: BTreeSet<NodeId> = [a, b].into();
        let at = Instant::now();
        let mut topology = Topology::new(node(1));

        topology.receive(
            network,
            stranger,
            vec![link(a, LinkPath::Direct)],
            &members,
            at,
        );
        topology.receive(
            network,
            local,
            vec![link(a, LinkPath::Direct)],
            &members,
            at,
        );
        topology.receive(
            network,
            a,
            vec![
                link(b, LinkPath::Direct),
                link(b, LinkPath::Relay),
                link(a, LinkPath::Direct),
                link(stranger, LinkPath::Direct),
                link(local, LinkPath::Relay),
            ],
            &members,
            at,
        );
        let reports: Vec<_> = topology.reports(network, at).collect();
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].2,
            [link(b, LinkPath::Direct), link(local, LinkPath::Relay)]
        );

        let later = at + Duration::from_secs(3);
        topology.receive(network, a, vec![link(b, LinkPath::Relay)], &members, later);
        topology.receive(other, b, vec![link(a, LinkPath::Direct)], &members, later);
        let reports: Vec<_> = topology.reports(network, later).collect();
        assert_eq!(
            reports,
            [(a, Duration::ZERO, &[link(b, LinkPath::Relay)][..])]
        );

        let shrunk = BTreeMap::from([(network, BTreeSet::from([b]))]);
        topology.retain(&shrunk, later);
        assert_eq!(topology.reports(network, later).count(), 0);
        assert_eq!(topology.reports(other, later).count(), 0);
    }

    #[test]
    fn a_report_is_sent_on_change_or_after_the_interval() {
        let network = NetworkId::from_bytes([9; 32]);
        let at = Instant::now();
        let mut topology = Topology::new(node(1));
        let links = vec![link(node(2), LinkPath::Direct)];
        assert!(topology.due(network, &links, at));
        topology.sent(network, links.clone(), at);

        let mut faster = links;
        faster[0].rtt_ms = Some(1);
        assert!(!topology.due(network, &faster, at + Duration::from_secs(1)));
        assert!(topology.due(network, &[link(node(2), LinkPath::Relay)], at));
        assert!(topology.due(network, &[], at));
        assert!(topology.due(network, &faster, at + REPORT_INTERVAL));
    }
}
