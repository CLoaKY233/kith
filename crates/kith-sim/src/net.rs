//! The simulated network: hosts, an event queue in virtual time, link chaos and partitions.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::nat::{Nat, NatConfig};
use crate::rng::Rng;

/// A simulated host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostId(usize);

/// How datagrams behave on every link. Streams only see `latency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// One-way base delay.
    pub latency: Duration,
    /// Extra uniform delay in `0..jitter`, which reorders nearby packets.
    pub jitter: Duration,
    /// Datagrams dropped, per thousand.
    pub loss_per_mille: u16,
    /// Datagrams delivered twice, per thousand.
    pub duplicate_per_mille: u16,
    /// Datagrams held back by up to four latencies, per thousand.
    pub reorder_per_mille: u16,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            latency: Duration::from_millis(20),
            jitter: Duration::ZERO,
            loss_per_mille: 0,
            duplicate_per_mille: 0,
            reorder_per_mille: 0,
        }
    }
}

/// Something arriving at a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// A UDP datagram, after NAT translation on both sides.
    Datagram {
        /// The receiving host.
        host: HostId,
        /// The source as the receiver sees it.
        from: SocketAddr,
        /// The receiver's local address it arrived on.
        to: SocketAddr,
        /// The bytes.
        payload: Vec<u8>,
    },
    /// Bytes on a reliable, ordered stream (TCP-like, unaffected by NAT and chaos).
    Stream {
        /// The receiving host.
        host: HostId,
        /// The sending host.
        from: HostId,
        /// The bytes.
        bytes: Vec<u8>,
    },
}

#[derive(Debug)]
struct Host {
    local: IpAddr,
    nat: Option<Nat>,
    /// `None` when reachable across every partition.
    group: Option<usize>,
    everywhere: bool,
}

#[derive(Debug)]
enum Pending {
    Datagram {
        to: HostId,
        from: SocketAddr,
        destination: SocketAddr,
        payload: Vec<u8>,
    },
    Stream {
        to: HostId,
        from: HostId,
        bytes: Vec<u8>,
    },
}

/// The simulation: virtual time, the seeded generator and the network.
#[derive(Debug)]
pub struct Sim {
    seed: u64,
    now: Duration,
    rng: Rng,
    link: LinkConfig,
    hosts: Vec<Host>,
    queue: BTreeMap<(Duration, u64), Pending>,
    sequence: u64,
    stream_clock: BTreeMap<(HostId, HostId), Duration>,
}

impl Sim {
    /// An empty network at time zero whose every random choice derives from `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            now: Duration::ZERO,
            rng: Rng::new(seed),
            link: LinkConfig::default(),
            hosts: Vec::new(),
            queue: BTreeMap::new(),
            sequence: 0,
            stream_clock: BTreeMap::new(),
        }
    }

    /// The seed that replays this run.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The current virtual time.
    pub const fn now(&self) -> Duration {
        self.now
    }

    /// The simulation's generator; use it for every random choice nodes make.
    pub const fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Sets the behaviour of every link from now on.
    pub const fn set_link(&mut self, link: LinkConfig) {
        self.link = link;
    }

    /// Adds a host with a public address, or behind its own NAT.
    pub fn add_host(&mut self, nat: Option<NatConfig>) -> HostId {
        self.push_host(nat, false)
    }

    /// Adds a public host that every partition can still reach, such as a relay.
    pub fn add_server(&mut self) -> HostId {
        self.push_host(None, true)
    }

    fn push_host(&mut self, nat: Option<NatConfig>, everywhere: bool) -> HostId {
        let index = u32::try_from(self.hosts.len()).unwrap_or(u32::MAX);
        let public = IpAddr::V4(Ipv4Addr::from(0xC633_6400_u32.wrapping_add(index)));
        let host = match nat {
            Some(config) => Host {
                local: IpAddr::V4(Ipv4Addr::from(0x0A00_0002_u32.wrapping_add(index))),
                nat: Some(Nat::new(config, public)),
                group: None,
                everywhere,
            },
            None => Host {
                local: public,
                nat: None,
                group: None,
                everywhere,
            },
        };
        self.hosts.push(host);
        HostId(self.hosts.len().saturating_sub(1))
    }

    /// The host's own address for `port`; behind a NAT this is private.
    pub fn address(&self, host: HostId, port: u16) -> Option<SocketAddr> {
        self.hosts
            .get(host.0)
            .map(|host| SocketAddr::new(host.local, port))
    }

    /// Splits the listed hosts into groups that cannot reach each other. Unlisted hosts and
    /// servers reach everyone.
    pub fn partition(&mut self, groups: &[&[HostId]]) {
        self.heal();
        for (group, members) in groups.iter().enumerate() {
            for member in *members {
                if let Some(host) = self.hosts.get_mut(member.0)
                    && !host.everywhere
                {
                    host.group = Some(group);
                }
            }
        }
    }

    /// Removes every partition.
    pub fn heal(&mut self) {
        for host in &mut self.hosts {
            host.group = None;
        }
    }

    fn reachable(&self, a: HostId, b: HostId) -> bool {
        match (self.hosts.get(a.0), self.hosts.get(b.0)) {
            (Some(a), Some(b)) => a.group.is_none() || b.group.is_none() || a.group == b.group,
            _ => false,
        }
    }

    fn route(&self, destination: IpAddr) -> Option<HostId> {
        self.hosts
            .iter()
            .position(|host| match &host.nat {
                Some(nat) => nat.public() == destination,
                None => host.local == destination,
            })
            .map(HostId)
    }

    /// Sends a datagram from `host`'s local `port`, through its NAT, subject to chaos.
    pub fn send_datagram(&mut self, host: HostId, port: u16, to: SocketAddr, payload: &[u8]) {
        let now = self.now;
        let Some(sender) = self.hosts.get_mut(host.0) else {
            return;
        };
        let local = SocketAddr::new(sender.local, port);
        let from = match &mut sender.nat {
            Some(nat) => nat.outbound(now, local, to),
            None => local,
        };
        let Some(receiver) = self.route(to.ip()) else {
            return;
        };
        if !self.reachable(host, receiver) || self.rng.chance(self.link.loss_per_mille) {
            return;
        }
        let copies = if self.rng.chance(self.link.duplicate_per_mille) {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let mut delay = self
                .link
                .latency
                .saturating_add(self.rng.duration_below(self.link.jitter));
            if self.rng.chance(self.link.reorder_per_mille) {
                let hold = self.link.latency.saturating_mul(4);
                delay = delay.saturating_add(self.rng.duration_below(hold));
            }
            self.schedule(
                delay,
                Pending::Datagram {
                    to: receiver,
                    from,
                    destination: to,
                    payload: payload.to_vec(),
                },
            );
        }
    }

    /// Sends bytes on the reliable stream from `from` to `to`. Bytes sent while the two
    /// are partitioned are lost.
    pub fn send_stream(&mut self, from: HostId, to: HostId, bytes: Vec<u8>) {
        if !self.reachable(from, to) {
            return;
        }
        let earliest = self.now.saturating_add(self.link.latency);
        let clock = self.stream_clock.entry((from, to)).or_default();
        let at = earliest.max(*clock);
        *clock = at;
        let delay = at.saturating_sub(self.now);
        self.schedule(delay, Pending::Stream { to, from, bytes });
    }

    fn schedule(&mut self, delay: Duration, pending: Pending) {
        self.sequence = self.sequence.saturating_add(1);
        self.queue
            .insert((self.now.saturating_add(delay), self.sequence), pending);
    }

    /// The next delivery due no later than `until`, advancing time to it. Returns `None`
    /// with time advanced to `until` once nothing more is due.
    pub fn next_delivery(&mut self, until: Duration) -> Option<Delivery> {
        while let Some(entry) = self.queue.first_entry() {
            if entry.key().0 > until {
                break;
            }
            let ((at, _), pending) = entry.remove_entry();
            self.now = self.now.max(at);
            match pending {
                Pending::Stream { to, from, bytes } => {
                    return Some(Delivery::Stream {
                        host: to,
                        from,
                        bytes,
                    });
                }
                Pending::Datagram {
                    to,
                    from,
                    destination,
                    payload,
                } => {
                    let now = self.now;
                    let Some(host) = self.hosts.get_mut(to.0) else {
                        continue;
                    };
                    let local = match &mut host.nat {
                        Some(nat) => nat.inbound(now, from, destination.port()),
                        None => Some(destination),
                    };
                    if let Some(local) = local {
                        return Some(Delivery::Datagram {
                            host: to,
                            from,
                            to: local,
                            payload,
                        });
                    }
                }
            }
        }
        self.now = self.now.max(until);
        None
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        if std::thread::panicking() {
            #[expect(clippy::print_stderr, reason = "the seed is how a failure is replayed")]
            {
                eprintln!(
                    "kith-sim: failed with seed {}; replay with Sim::new({0})",
                    self.seed
                );
            }
        }
    }
}
