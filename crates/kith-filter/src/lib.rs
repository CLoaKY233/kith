//! Stateful packet filter that evaluates compiled policy rules.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # Why this crate exists
//!
//! The policy ([`kith_policy::CompiledFilter`]) says which peers may open traffic *to*
//! this device. That alone would break every connection this device opens to a peer
//! the policy does not trust: the replies would be dropped. [`Filter`] adds connection
//! tracking so replies to our own flows come back in.
//!
//! - **Outbound** traffic is always allowed: each remote enforces its own inbound
//!   policy. Outbound packets that open a flow are recorded.
//! - **Inbound** traffic is accepted if it is the reply direction of a live tracked
//!   flow, or if the policy allows it. Everything else is dropped.
//!
//! # What is tracked
//!
//! - **TCP** flows start only on an outbound SYN without ACK. Mid-stream outbound
//!   packets with no tracked flow (after a restart, or an eviction) are not picked up:
//!   otherwise replying to a peer the policy no longer admits would reopen the door for
//!   it. States are SYN sent, established, closing (FIN) and closed (RST).
//! - **UDP** flows start on any outbound datagram and become a stream once a reply is
//!   seen. A UDP reply to an inbound-allowed datagram therefore keeps that flow open.
//! - **ICMP echo** requests are tracked by identifier; only an echo *reply* matches.
//!
//! Inbound ICMP errors, and protocols other than TCP, UDP and ICMP, are judged by the
//! policy alone. Timeouts follow Linux conntrack; see [`timeouts`].
//!
//! # Fragments and malformed packets
//!
//! Inbound IP fragments are dropped as [`DropReason::Malformed`]: an IPv4 later
//! fragment carries no ports, so it can be matched neither against a flow nor against
//! port rules. That covers IPv4 packets with the more-fragments flag or a non-zero
//! offset, and IPv6 packets whose first next header is a Fragment header. The tunnel
//! MTU makes fragments rare; senders should use path MTU discovery. Inbound packets
//! that do not parse are dropped the same way. Outbound fragments and unparsable
//! packets are passed through untracked.
//!
//! # Memory and performance
//!
//! The connection table holds at most a fixed number of flows ([`DEFAULT_MAX_ENTRIES`],
//! or [`Filter::with_capacity`]) and allocates all its memory up front. Per packet the
//! filter allocates nothing. When the table is full, the soonest-expiring flow that
//! never became two-way, near the new flow's hash slot, is evicted. If there is none,
//! the new outbound flow is dropped with [`DropReason::TableFull`], as Linux does:
//! established connections are never sacrificed to new ones.
//!
//! The hash is fast and unkeyed. Only outbound packets create flows, so a remote peer
//! cannot fill the table, and since the table is bounded, deliberate collisions cost
//! longer probes and evictions, never memory. The policy still applies to every
//! packet that does not match a flow.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod table;
pub mod timeouts;

#[cfg(test)]
mod tests;

use core::net::IpAddr;
use core::time::Duration;

use kith_packet::{FlowKey, IcmpPacket, IpPacket, TcpFlags, TcpPacket, ip_protocol};
use kith_policy::{CompiledFilter, PacketProtocol};

use table::{ConnectionKey, Entry, Full, Inserted, State, Table};

/// The default connection-table size: 8192 flows, about 1 MiB of slots.
///
/// Sized for a small router (OpenWrt sets `nf_conntrack_max` to 16384 on most
/// devices, and the mesh carries only a share of a router's traffic).
pub const DEFAULT_MAX_ENTRIES: usize = 8192;

/// The largest accepted table size (2^24 flows); larger requests are clamped.
pub const MAX_ENTRIES: usize = table::MAX_ENTRIES_LIMIT;

/// IPv6 Fragment extension header (RFC 8200 §4.5).
const IPV6_FRAGMENT: u8 = 44;

/// ICMP echo message types (RFC 792, RFC 4443 §4).
const ICMPV4_ECHO_REPLY: u8 = 0;
const ICMPV4_ECHO_REQUEST: u8 = 8;
const ICMPV6_ECHO_REQUEST: u8 = 128;
const ICMPV6_ECHO_REPLY: u8 = 129;

/// What to do with a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Let the packet through.
    Accept,
    /// Discard the packet.
    Drop(DropReason),
}

/// Why a packet was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropReason {
    /// Inbound, matches no tracked flow, and the policy does not allow it.
    Policy,
    /// Inbound, and unparsable or an IP fragment.
    Malformed,
    /// Outbound, opens a new flow, and the full table has nothing evictable.
    TableFull,
}

/// Running totals, for metrics. Every counter saturates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Packets dropped with [`DropReason::Policy`].
    pub dropped_policy: u64,
    /// Packets dropped with [`DropReason::Malformed`].
    pub dropped_malformed: u64,
    /// Packets dropped with [`DropReason::TableFull`].
    pub dropped_table_full: u64,
    /// Live flows evicted to make room for new ones.
    pub evicted: u64,
}

/// The stateful filter for one device on one network.
///
/// `now` is a monotonic time supplied by the driver; it must not go backwards.
#[derive(Debug)]
pub struct Filter {
    policy: CompiledFilter,
    table: Table,
    counters: Counters,
}

impl Filter {
    /// A filter enforcing `policy`, with room for [`DEFAULT_MAX_ENTRIES`] flows.
    pub fn new(policy: CompiledFilter) -> Self {
        Self::with_capacity(policy, DEFAULT_MAX_ENTRIES)
    }

    /// A filter enforcing `policy`, with room for `max_entries` flows (clamped to
    /// `1..=`[`MAX_ENTRIES`]). All table memory is allocated here.
    pub fn with_capacity(policy: CompiledFilter, max_entries: usize) -> Self {
        Self {
            policy,
            table: Table::new(max_entries),
            counters: Counters::default(),
        }
    }

    /// Replaces the policy. Tracked flows are kept, so their replies still pass.
    pub fn set_policy(&mut self, policy: CompiledFilter) {
        self.policy = policy;
    }

    /// The policy in force.
    pub const fn policy(&self) -> &CompiledFilter {
        &self.policy
    }

    /// Drop and eviction totals since creation.
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// Flows in the table, including stale ones not yet removed by [`Filter::expire`].
    pub const fn len(&self) -> usize {
        self.table.len()
    }

    /// Whether the table holds no flows.
    pub const fn is_empty(&self) -> bool {
        self.table.len() == 0
    }

    /// The most flows the table holds.
    pub const fn capacity(&self) -> usize {
        self.table.max_entries()
    }

    /// Removes flows idle past their timeout. Call periodically, e.g. every few seconds;
    /// lookups already ignore stale flows, so this only frees slots early.
    pub fn expire(&mut self, now: Duration) {
        self.table.expire(now);
    }

    /// Judges a packet this device sends, and records the flow it opens.
    ///
    /// Always [`Verdict::Accept`], unless the packet opens a flow and the table is full
    /// ([`DropReason::TableFull`]).
    pub fn outbound(&mut self, packet: &[u8], now: Duration) -> Verdict {
        let Some(parsed) = Parsed::new(packet) else {
            return Verdict::Accept;
        };
        let (state, key) = match parsed.transport {
            Transport::Tcp(flags) => {
                let key = parsed.outbound_key();
                if let Some(entry) = self.table.get_mut(&key, now) {
                    let state = tcp_next(entry.state, flags, Direction::Outbound);
                    entry.refresh(state, now);
                    return Verdict::Accept;
                }
                if !is_opening_syn(flags) {
                    return Verdict::Accept;
                }
                (State::TcpSynSent, key)
            }
            Transport::Udp => (State::UdpUnreplied, parsed.outbound_key()),
            Transport::EchoRequest(identifier) => (State::IcmpEcho, parsed.echo_key(identifier)),
            Transport::EchoReply(_) | Transport::OtherIcmp | Transport::Other => {
                return Verdict::Accept;
            }
        };
        if let Some(entry) = self.table.get_mut(&key, now) {
            // UDP and ICMP: outbound traffic only keeps the flow alive.
            let state = entry.state;
            entry.refresh(state, now);
            return Verdict::Accept;
        }
        match self.table.insert(Entry::new(key, state, now), now) {
            Ok(Inserted::Fresh) => Verdict::Accept,
            Ok(Inserted::Evicted) => {
                self.counters.evicted = self.counters.evicted.saturating_add(1);
                Verdict::Accept
            }
            Err(Full) => self.drop(DropReason::TableFull),
        }
    }

    /// Judges a packet arriving for this device.
    ///
    /// Accepts the reply direction of a live tracked flow, then anything the policy
    /// allows; drops the rest.
    pub fn inbound(&mut self, packet: &[u8], now: Duration) -> Verdict {
        let Some(parsed) = Parsed::new(packet) else {
            return self.drop(DropReason::Malformed);
        };
        let reply_key = match parsed.transport {
            Transport::Tcp(_) | Transport::Udp => Some(parsed.reply_key()),
            Transport::EchoReply(identifier) => Some(parsed.reply_echo_key(identifier)),
            Transport::EchoRequest(_) | Transport::OtherIcmp | Transport::Other => None,
        };
        if let Some(entry) = reply_key.and_then(|key| self.table.get_mut(&key, now)) {
            let state = match parsed.transport {
                Transport::Tcp(flags) => tcp_next(entry.state, flags, Direction::Inbound),
                Transport::Udp => State::UdpStream,
                Transport::EchoReply(_)
                | Transport::EchoRequest(_)
                | Transport::OtherIcmp
                | Transport::Other => entry.state,
            };
            entry.refresh(state, now);
            return Verdict::Accept;
        }
        let port = parsed.has_ports.then_some(parsed.destination_port);
        if self.policy.allows(parsed.source, parsed.protocol, port) {
            Verdict::Accept
        } else {
            self.drop(DropReason::Policy)
        }
    }

    fn drop(&mut self, reason: DropReason) -> Verdict {
        let counter = match reason {
            DropReason::Policy => &mut self.counters.dropped_policy,
            DropReason::Malformed => &mut self.counters.dropped_malformed,
            DropReason::TableFull => &mut self.counters.dropped_table_full,
        };
        *counter = counter.saturating_add(1);
        Verdict::Drop(reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Outbound,
    Inbound,
}

/// A SYN that opens a connection, as opposed to the SYN-ACK answering one.
const fn is_opening_syn(flags: TcpFlags) -> bool {
    flags.contains(TcpFlags::SYN) && !flags.contains(TcpFlags::ACK)
}

/// The TCP state after a packet with `flags` travels in `direction`.
///
/// Deliberately coarse, like conntrack without window tracking: an RST closes, a FIN
/// starts closing, any reply establishes, and a fresh SYN reopens a closed flow.
fn tcp_next(state: State, flags: TcpFlags, direction: Direction) -> State {
    if flags.contains(TcpFlags::RST) {
        State::TcpClosed
    } else if flags.contains(TcpFlags::FIN) {
        if state == State::TcpClosed {
            State::TcpClosed
        } else {
            State::TcpClosing
        }
    } else {
        match (state, direction) {
            (State::TcpSynSent, Direction::Inbound) => State::TcpEstablished,
            (State::TcpClosing | State::TcpClosed, Direction::Outbound)
                if is_opening_syn(flags) =>
            {
                State::TcpSynSent
            }
            _ => state,
        }
    }
}

/// The transport details the filter needs.
#[derive(Debug, Clone, Copy)]
enum Transport {
    Tcp(TcpFlags),
    Udp,
    /// Echo request, by identifier.
    EchoRequest(u16),
    /// Echo reply, by identifier.
    EchoReply(u16),
    OtherIcmp,
    Other,
}

/// One packet, parsed once.
#[derive(Debug, Clone, Copy)]
struct Parsed {
    source: IpAddr,
    destination: IpAddr,
    protocol_number: u8,
    protocol: PacketProtocol,
    transport: Transport,
    /// Whether the ports below are real (TCP and UDP).
    has_ports: bool,
    source_port: u16,
    destination_port: u16,
}

impl Parsed {
    /// Parses an IP packet; `None` for anything malformed and for fragments.
    fn new(packet: &[u8]) -> Option<Self> {
        let ip = IpPacket::new_checked(packet).ok()?;
        let is_fragment = match &ip {
            IpPacket::V4(v4) => v4.more_fragments() || v4.fragment_offset() != 0,
            IpPacket::V6(v6) => v6.next_header() == IPV6_FRAGMENT,
        };
        if is_fragment {
            return None;
        }
        let flow = FlowKey::extract(packet).ok()?;
        // (protocol, echo request type, echo reply type) for the packet's family.
        let (icmp, echo_request, echo_reply) = match ip {
            IpPacket::V4(_) => (ip_protocol::ICMP, ICMPV4_ECHO_REQUEST, ICMPV4_ECHO_REPLY),
            IpPacket::V6(_) => (ip_protocol::ICMPV6, ICMPV6_ECHO_REQUEST, ICMPV6_ECHO_REPLY),
        };
        let (protocol, transport) = match flow.protocol {
            ip_protocol::TCP => {
                let tcp = TcpPacket::new_checked(ip.payload()).ok()?;
                (PacketProtocol::Tcp, Transport::Tcp(tcp.flags()))
            }
            ip_protocol::UDP => (PacketProtocol::Udp, Transport::Udp),
            number if number == icmp => {
                let message = IcmpPacket::new_checked(ip.payload()).ok()?;
                // Echo messages carry the identifier first (RFC 792, RFC 4443 §4.1).
                let identifier = match message.payload() {
                    [high, low, ..] => Some(u16::from_be_bytes([*high, *low])),
                    _ => None,
                };
                let message_type = message.message_type();
                let transport = if message_type == echo_request {
                    Transport::EchoRequest(identifier?)
                } else if message_type == echo_reply {
                    Transport::EchoReply(identifier?)
                } else {
                    Transport::OtherIcmp
                };
                (PacketProtocol::Icmp, transport)
            }
            number => (PacketProtocol::Other(number), Transport::Other),
        };
        let has_ports = matches!(transport, Transport::Tcp(_) | Transport::Udp);
        let (source_port, destination_port) = match (flow.src_port, flow.dst_port) {
            (Some(source), Some(destination)) => (source, destination),
            // TCP and UDP outside a fragment always carry ports.
            _ if has_ports => return None,
            _ => (0, 0),
        };
        Some(Self {
            source: flow.src,
            destination: flow.dst,
            protocol_number: flow.protocol,
            protocol,
            transport,
            has_ports,
            source_port,
            destination_port,
        })
    }

    /// The flow as opened by this device sending this packet.
    const fn outbound_key(&self) -> ConnectionKey {
        ConnectionKey {
            local: self.source,
            remote: self.destination,
            protocol: self.protocol_number,
            local_port: self.source_port,
            remote_port: self.destination_port,
        }
    }

    /// The flow this inbound packet would be a reply to.
    const fn reply_key(&self) -> ConnectionKey {
        ConnectionKey {
            local: self.destination,
            remote: self.source,
            protocol: self.protocol_number,
            local_port: self.destination_port,
            remote_port: self.source_port,
        }
    }

    const fn echo_key(&self, identifier: u16) -> ConnectionKey {
        ConnectionKey {
            local: self.source,
            remote: self.destination,
            protocol: self.protocol_number,
            local_port: identifier,
            remote_port: 0,
        }
    }

    const fn reply_echo_key(&self, identifier: u16) -> ConnectionKey {
        ConnectionKey {
            local: self.destination,
            remote: self.source,
            protocol: self.protocol_number,
            local_port: identifier,
            remote_port: 0,
        }
    }
}
