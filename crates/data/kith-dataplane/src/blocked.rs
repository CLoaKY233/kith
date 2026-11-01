//! A bounded sample of the inbound connections the policy refused, so a user can see what
//! was blocked and allow it.
//!
//! The table is allocated once and never grows. Samples are rate limited, so a flood of
//! refused packets costs a counter bump and one comparison each.

use std::net::IpAddr;
use std::time::Duration;

use kith_types::NetworkId;

/// The most distinct connections kept; the one seen longest ago makes room for a new one.
pub(crate) const MAX_BLOCKED: usize = 256;

/// The most refused packets sampled per [`WINDOW`]; the rest are only counted as drops.
const SAMPLES_PER_WINDOW: u32 = 512;
const WINDOW: Duration = Duration::from_secs(1);

const TCP: u8 = 6;
const UDP: u8 = 17;

/// Inbound packets of one connection the network's policy refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Blocked {
    /// The network whose policy refused them.
    pub network: NetworkId,
    /// Who sent them.
    pub source: IpAddr,
    /// The address they were for.
    pub destination: IpAddr,
    /// The IP protocol number: 6 is TCP, 17 UDP, 1 and 58 ICMP.
    pub protocol: u8,
    /// The destination port, for TCP and UDP.
    pub port: Option<u16>,
    /// Packets sampled. Under a flood only some are sampled, so this can be lower than
    /// the number refused.
    pub count: u64,
}

impl Blocked {
    fn same_connection(&self, other: &Self) -> bool {
        self.network == other.network
            && self.source == other.source
            && self.destination == other.destination
            && self.protocol == other.protocol
            && self.port == other.port
    }
}

#[derive(Debug)]
struct Slot {
    blocked: Blocked,
    last_seen: Duration,
}

/// The refused connections seen, at most [`MAX_BLOCKED`].
#[derive(Debug)]
pub(crate) struct BlockedTable {
    slots: Vec<Slot>,
    window_start: Duration,
    sampled_in_window: u32,
}

impl Default for BlockedTable {
    fn default() -> Self {
        Self {
            slots: Vec::with_capacity(MAX_BLOCKED),
            window_start: Duration::ZERO,
            sampled_in_window: 0,
        }
    }
}

impl BlockedTable {
    /// Notes that `network`'s policy refused `packet` at `now`.
    pub(crate) fn record(&mut self, network: NetworkId, packet: &[u8], now: Duration) {
        if now.saturating_sub(self.window_start) >= WINDOW {
            self.window_start = now;
            self.sampled_in_window = 0;
        }
        if self.sampled_in_window >= SAMPLES_PER_WINDOW {
            return;
        }
        self.sampled_in_window = self.sampled_in_window.saturating_add(1);
        let Some(sample) = parse(network, packet) else {
            return;
        };
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.blocked.same_connection(&sample))
        {
            slot.blocked.count = slot.blocked.count.saturating_add(1);
            slot.last_seen = now;
            return;
        }
        let fresh = Slot {
            blocked: sample,
            last_seen: now,
        };
        if self.slots.len() < MAX_BLOCKED {
            self.slots.push(fresh);
        } else if let Some(oldest) = self.slots.iter_mut().min_by_key(|slot| slot.last_seen) {
            *oldest = fresh;
        }
    }

    /// Every connection kept.
    pub(crate) fn snapshot(&self) -> Vec<Blocked> {
        self.slots.iter().map(|slot| slot.blocked).collect()
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.slots.capacity()
    }
}

fn parse(network: NetworkId, packet: &[u8]) -> Option<Blocked> {
    let source = crate::packet::source(packet)?;
    let destination = crate::packet::destination(packet)?;
    let (protocol, transport) = match packet.first()? >> 4 {
        4 => {
            let header = usize::from(packet.first()? & 0x0f) * 4;
            (*packet.get(9)?, header)
        }
        _ => (*packet.get(6)?, 40),
    };
    let port = matches!(protocol, TCP | UDP)
        .then(|| {
            let bytes = packet.get(transport + 2..transport + 4)?;
            Some(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]))
        })
        .flatten();
    Some(Blocked {
        network,
        source,
        destination,
        protocol,
        port,
        count: 1,
    })
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn udp(source: [u8; 4], port: u16) -> Vec<u8> {
        let mut packet = vec![0x45, 0, 0, 28, 0, 0, 0, 0, 64, UDP, 0, 0];
        packet.extend_from_slice(&source);
        packet.extend_from_slice(&[100, 64, 0, 1]);
        packet.extend_from_slice(&[0x30, 0x39]);
        packet.extend_from_slice(&port.to_be_bytes());
        packet.extend_from_slice(&[0, 8, 0, 0]);
        packet
    }

    #[test]
    fn a_flood_of_refused_packets_stays_bounded_and_aggregated() {
        let network = NetworkId::from_bytes([1; 32]);
        let mut table = BlockedTable::default();
        let mut now = Duration::ZERO;
        let ssh = |table: &BlockedTable| {
            table
                .snapshot()
                .into_iter()
                .find(|b| b.source == IpAddr::from(Ipv4Addr::new(100, 64, 0, 7)))
        };
        for _ in 0..20 {
            now += WINDOW;
            for port in 0..400u16 {
                table.record(network, &udp([100, 64, 0, 9], 1000 + port % 200), now);
            }
            table.record(network, &udp([100, 64, 0, 7], 22), now);
        }
        let kept = table.snapshot();
        assert_eq!(kept.len(), 201);
        assert!(kept.iter().all(|b| b.protocol == UDP));
        assert_eq!(ssh(&table).map(|b| (b.port, b.count)), Some((Some(22), 20)));
        assert_eq!(kept.iter().map(|b| b.count).sum::<u64>(), 20 * 401);

        now += WINDOW;
        for _ in 0..100_000 {
            table.record(network, &udp([100, 64, 0, 7], 22), now);
        }
        assert_eq!(
            ssh(&table).map(|b| b.count),
            Some(20 + u64::from(SAMPLES_PER_WINDOW)),
            "sampling is rate limited"
        );

        for port in 0..u16::MAX {
            now += WINDOW / 4;
            table.record(network, &udp([100, 64, 0, 8], port), now);
        }
        assert_eq!(table.snapshot().len(), MAX_BLOCKED);
        assert_eq!(table.capacity(), MAX_BLOCKED, "the table never grows");
        assert!(ssh(&table).is_none(), "the oldest connections make room");
    }
}
