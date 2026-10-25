//! Filter behaviour: hand-written cases, and agreement with a naive reference model.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "test arithmetic on small constants and simulated time cannot overflow"
)]

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::time::Duration;

use kith_packet::TcpFlags;
use kith_policy::{
    Addresses, CompiledFilter, Member, Members, PacketProtocol, Policy, PortSet, Protocol, Rule,
    Selector, Target, compile,
};
use kith_types::NodeId;
use proptest::prelude::*;

use crate::{DropReason, Filter, Verdict};

const LOCAL: u8 = 0;
const PEERS: u8 = 3;
const SYN: u8 = TcpFlags::SYN.bits();
const SYN_ACK: u8 = TcpFlags::SYN.bits() | TcpFlags::ACK.bits();
const ACK: u8 = TcpFlags::ACK.bits();
const FIN: u8 = TcpFlags::FIN.bits() | TcpFlags::ACK.bits();
const RST: u8 = TcpFlags::RST.bits();

fn node(index: u8) -> NodeId {
    NodeId::from_bytes([index; 32])
}

fn addresses(index: u8) -> Addresses {
    Addresses {
        v4: Ipv4Addr::new(100, 64, 0, index + 2),
        v6: Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, u16::from(index) + 2),
    }
}

fn v4(index: u8) -> IpAddr {
    IpAddr::V4(addresses(index).v4)
}

fn v6(index: u8) -> IpAddr {
    IpAddr::V6(addresses(index).v6)
}

/// Compiles `policy` for the local device, in a network of `LOCAL` plus `PEERS` peers.
fn compiled(policy: &Policy) -> CompiledFilter {
    let mut members = Members::new();
    for index in 0..=PEERS {
        let member = Member {
            tags: BTreeSet::new(),
            addresses: addresses(index),
        };
        members.insert(node(index), member);
    }
    compile(policy, &members, node(LOCAL))
}

/// Peer 1 may reach the local device on TCP 22; nothing else is allowed.
fn ssh_from_peer_one() -> Policy {
    let ssh = Target::new(
        Selector::Device(node(LOCAL)),
        Protocol::Tcp,
        PortSet::single(22),
    )
    .unwrap();
    Policy::new(vec![
        Rule::new(vec![Selector::Device(node(1))], vec![ssh]).unwrap(),
    ])
    .unwrap()
}

fn policies() -> [Policy; 3] {
    [
        Policy::deny_all(),
        Policy::default_allow_all(),
        ssh_from_peer_one(),
    ]
}

/// The transport part of a test packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    Tcp {
        source: u16,
        destination: u16,
        flags: u8,
    },
    Udp {
        source: u16,
        destination: u16,
    },
    Echo {
        reply: bool,
        identifier: u16,
    },
}

impl Probe {
    /// The same probe travelling the other way.
    const fn reversed(self) -> Self {
        match self {
            Self::Tcp {
                source,
                destination,
                flags,
            } => Self::Tcp {
                source: destination,
                destination: source,
                flags,
            },
            Self::Udp {
                source,
                destination,
            } => Self::Udp {
                source: destination,
                destination: source,
            },
            Self::Echo { reply, identifier } => Self::Echo {
                reply: !reply,
                identifier,
            },
        }
    }
}

const fn tcp(source: u16, destination: u16, flags: u8) -> Probe {
    Probe::Tcp {
        source,
        destination,
        flags,
    }
}

/// Builds a minimal, well-formed IPv4 or IPv6 packet (checksums left zero).
fn packet(source: IpAddr, destination: IpAddr, probe: Probe) -> Vec<u8> {
    let is_v4 = source.is_ipv4();
    let (protocol, transport) = match probe {
        Probe::Tcp {
            source,
            destination,
            flags,
        } => {
            let mut bytes = Vec::new();
            bytes.extend(source.to_be_bytes());
            bytes.extend(destination.to_be_bytes());
            bytes.extend([0; 8]); // sequence and acknowledgement numbers
            bytes.extend([0x50, flags, 0xff, 0xff, 0, 0, 0, 0]);
            (6, bytes)
        }
        Probe::Udp {
            source,
            destination,
        } => {
            let mut bytes = Vec::new();
            bytes.extend(source.to_be_bytes());
            bytes.extend(destination.to_be_bytes());
            bytes.extend([0, 8, 0, 0]);
            (17, bytes)
        }
        Probe::Echo { reply, identifier } => {
            let (protocol, message_type) = match (is_v4, reply) {
                (true, false) => (1, 8),
                (true, true) => (1, 0),
                (false, false) => (58, 128),
                (false, true) => (58, 129),
            };
            let mut bytes = vec![message_type, 0, 0, 0];
            bytes.extend(identifier.to_be_bytes());
            bytes.extend([0, 1]);
            (protocol, bytes)
        }
    };
    let mut bytes = Vec::new();
    match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            let total = u16::try_from(20 + transport.len()).unwrap();
            bytes.extend([0x45, 0]);
            bytes.extend(total.to_be_bytes());
            bytes.extend([0, 0, 0, 0, 64, protocol, 0, 0]);
            bytes.extend(source.octets());
            bytes.extend(destination.octets());
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            let payload = u16::try_from(transport.len()).unwrap();
            bytes.extend([0x60, 0, 0, 0]);
            bytes.extend(payload.to_be_bytes());
            bytes.extend([protocol, 64]);
            bytes.extend(source.octets());
            bytes.extend(destination.octets());
        }
        _ => panic!("mixed address families"),
    }
    bytes.extend(transport);
    bytes
}

const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

const POLICY: Verdict = Verdict::Drop(DropReason::Policy);

#[test]
fn tcp_reply_passes_a_denying_policy_but_unsolicited_traffic_does_not() {
    let mut filter = Filter::new(compiled(&Policy::deny_all()));
    let syn = tcp(40000, 22, SYN);
    assert_eq!(
        filter.outbound(&packet(v4(0), v4(1), syn), secs(0)),
        Verdict::Accept
    );

    let reply = packet(v4(1), v4(0), tcp(22, 40000, SYN_ACK));
    assert_eq!(filter.inbound(&reply, secs(1)), Verdict::Accept);
    let other_port = packet(v4(1), v4(0), tcp(22, 40001, SYN_ACK));
    assert_eq!(filter.inbound(&other_port, secs(1)), POLICY);
    assert_eq!(
        filter.inbound(&packet(v4(2), v4(0), tcp(22, 40000, ACK)), secs(1)),
        POLICY
    );

    // A mid-stream outbound packet opens nothing: answering a peer must not let it in.
    let answer = packet(v4(0), v4(1), tcp(40002, 22, ACK));
    assert_eq!(filter.outbound(&answer, secs(1)), Verdict::Accept);
    assert_eq!(
        filter.inbound(&packet(v4(1), v4(0), tcp(22, 40002, ACK)), secs(1)),
        POLICY
    );

    let mut fragment = reply;
    fragment[6] = 0x20; // more fragments
    assert_eq!(
        filter.inbound(&fragment, secs(1)),
        Verdict::Drop(DropReason::Malformed)
    );
    assert_eq!(filter.counters().dropped_policy, 3);
    assert_eq!(filter.counters().dropped_malformed, 1);
}

#[test]
fn udp_and_icmp_replies_pass_only_for_their_own_flow() {
    let udp = Probe::Udp {
        source: 5000,
        destination: 53,
    };
    let echo = Probe::Echo {
        reply: false,
        identifier: 7,
    };
    // (local, peer, outbound request, packets from the peer that must be dropped)
    let cases = [
        (
            v4(0),
            v4(1),
            udp,
            Probe::Udp {
                source: 53,
                destination: 5001,
            },
        ),
        (
            v6(0),
            v6(1),
            udp,
            Probe::Udp {
                source: 54,
                destination: 5000,
            },
        ),
        // An echo *request* with our identifier is not a reply.
        (v4(0), v4(1), echo, echo),
        (
            v6(0),
            v6(1),
            echo,
            Probe::Echo {
                reply: true,
                identifier: 8,
            },
        ),
    ];
    for (local, peer, request, unsolicited) in cases {
        let mut filter = Filter::new(compiled(&Policy::deny_all()));
        assert_eq!(
            filter.outbound(&packet(local, peer, request), secs(0)),
            Verdict::Accept
        );
        let reply = packet(peer, local, request.reversed());
        assert_eq!(
            filter.inbound(&reply, secs(1)),
            Verdict::Accept,
            "{request:?}"
        );
        let stranger = packet(peer, local, unsolicited);
        assert_eq!(
            filter.inbound(&stranger, secs(1)),
            POLICY,
            "{unsolicited:?}"
        );
    }
}

#[test]
fn flows_expire_and_fin_or_rst_shortens_them() {
    let mut filter = Filter::new(compiled(&Policy::deny_all()));
    let open = |filter: &mut Filter, port: u16| {
        filter.outbound(&packet(v4(0), v4(1), tcp(port, 80, SYN)), secs(0));
        let established = filter.inbound(&packet(v4(1), v4(0), tcp(80, port, SYN_ACK)), secs(0));
        assert_eq!(established, Verdict::Accept);
    };
    let reply = |port: u16| packet(v4(1), v4(0), tcp(80, port, ACK));

    // Established: idle for just under two hours is fine, and traffic refreshes it.
    open(&mut filter, 1);
    assert_eq!(filter.inbound(&reply(1), secs(7199)), Verdict::Accept);
    assert_eq!(filter.inbound(&reply(1), secs(14398)), Verdict::Accept);
    assert_eq!(filter.inbound(&reply(1), secs(21598)), POLICY);

    open(&mut filter, 2);
    filter.outbound(&packet(v4(0), v4(1), tcp(2, 80, RST)), secs(0));
    assert_eq!(filter.inbound(&reply(2), secs(9)), Verdict::Accept);
    assert_eq!(filter.inbound(&reply(2), secs(19)), POLICY);
    open(&mut filter, 3);
    filter.inbound(&packet(v4(1), v4(0), tcp(80, 3, FIN)), secs(0));
    assert_eq!(filter.inbound(&reply(3), secs(119)), Verdict::Accept);
    assert_eq!(filter.inbound(&reply(3), secs(239)), POLICY);

    let udp = Probe::Udp {
        source: 4,
        destination: 80,
    };
    filter.outbound(&packet(v4(0), v4(1), udp), secs(0));
    assert_eq!(
        filter.inbound(&packet(v4(1), v4(0), udp.reversed()), secs(30)),
        POLICY
    );

    assert_eq!(filter.len(), 4);
    filter.expire(secs(21598));
    assert!(filter.is_empty());
}

#[test]
fn full_table_evicts_only_unanswered_flows_and_never_grows() {
    let mut filter = Filter::with_capacity(compiled(&Policy::deny_all()), 4);
    let syn = |port: u16| packet(v4(0), v4(1), tcp(port, 80, SYN));
    let reply = |port: u16| packet(v4(1), v4(0), tcp(80, port, ACK));

    // Four established connections fill the table; a fifth is refused, not swapped in.
    for port in 1..=4 {
        assert_eq!(filter.outbound(&syn(port), secs(0)), Verdict::Accept);
        assert_eq!(filter.inbound(&reply(port), secs(0)), Verdict::Accept);
    }
    assert_eq!(
        filter.outbound(&syn(5), secs(1)),
        Verdict::Drop(DropReason::TableFull)
    );
    for port in 1..=4 {
        assert_eq!(filter.inbound(&reply(port), secs(1)), Verdict::Accept);
    }

    // Once they go stale, their slots are reused without counting as evictions.
    let later = secs(1 + 7200);
    for port in 11..=14 {
        assert_eq!(filter.outbound(&syn(port), later), Verdict::Accept);
    }
    assert_eq!(filter.counters().evicted, 0);

    // Unanswered SYNs are fair game: the oldest goes, the newest is tracked.
    assert_eq!(filter.outbound(&syn(15), later + secs(1)), Verdict::Accept);
    assert_eq!(filter.counters().evicted, 1);
    assert_eq!(filter.len(), filter.capacity());
    assert_eq!(filter.inbound(&reply(15), later + secs(1)), Verdict::Accept);
    let survivors = (11..=14)
        .filter(|&port| filter.inbound(&reply(port), later + secs(1)) == Verdict::Accept)
        .count();
    assert_eq!(survivors, 3);
}

#[test]
fn policy_swap_keeps_tracked_flows_and_applies_the_new_rules() {
    let mut filter = Filter::new(compiled(&Policy::default_allow_all()));
    filter.outbound(&packet(v4(0), v4(1), tcp(40000, 443, SYN)), secs(0));
    filter.set_policy(compiled(&Policy::deny_all()));
    let reply = packet(v4(1), v4(0), tcp(443, 40000, SYN_ACK));
    assert_eq!(filter.inbound(&reply, secs(1)), Verdict::Accept);
    let unsolicited = packet(v4(1), v4(0), tcp(443, 40001, SYN));
    assert_eq!(filter.inbound(&unsolicited, secs(1)), POLICY);
}

/// The reference model: a flat list of flows, scanned linearly, with the timeouts
/// written out as plain numbers.
#[derive(Debug, Default)]
struct Model {
    /// (local, remote, outbound probe that opened it, state, expires at)
    flows: Vec<(IpAddr, IpAddr, FlowId, ModelState, Duration)>,
}

/// What identifies a flow, from the opener's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowId {
    Tcp(u16, u16),
    Udp(u16, u16),
    Echo(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelState {
    SynSent,
    Established,
    Closing,
    Closed,
    UdpOneWay,
    UdpTwoWay,
    Echo,
}

impl ModelState {
    const fn seconds(self) -> u64 {
        match self {
            Self::SynSent | Self::Closing | Self::UdpTwoWay => 120,
            Self::Established => 7200,
            Self::Closed => 10,
            Self::UdpOneWay | Self::Echo => 30,
        }
    }
}

fn model_tcp(state: ModelState, flags: u8, outbound: bool) -> ModelState {
    let has = |flag: TcpFlags| flags & flag.bits() != 0;
    if has(TcpFlags::RST) || (state == ModelState::Closed && has(TcpFlags::FIN)) {
        ModelState::Closed
    } else if has(TcpFlags::FIN) {
        ModelState::Closing
    } else if state == ModelState::SynSent && !outbound {
        ModelState::Established
    } else if matches!(state, ModelState::Closing | ModelState::Closed)
        && outbound
        && has(TcpFlags::SYN)
        && !has(TcpFlags::ACK)
    {
        ModelState::SynSent
    } else {
        state
    }
}

impl Model {
    fn purge(&mut self, now: Duration) {
        self.flows.retain(|flow| flow.4 > now);
    }

    /// Index of the live flow from `local` to `remote` identified by `id`.
    fn find(&mut self, local: IpAddr, remote: IpAddr, id: FlowId, now: Duration) -> Option<usize> {
        self.purge(now);
        self.flows
            .iter()
            .position(|flow| flow.0 == local && flow.1 == remote && flow.2 == id)
    }

    fn touch(&mut self, index: usize, state: ModelState, now: Duration) {
        self.flows[index].3 = state;
        self.flows[index].4 = now + secs(state.seconds());
    }

    fn outbound(&mut self, local: IpAddr, remote: IpAddr, probe: Probe, now: Duration) {
        let (id, initial) = match probe {
            Probe::Tcp {
                source,
                destination,
                flags,
            } => {
                let id = FlowId::Tcp(source, destination);
                if let Some(index) = self.find(local, remote, id, now) {
                    let state = model_tcp(self.flows[index].3, flags, true);
                    return self.touch(index, state, now);
                }
                if flags & SYN == 0 || flags & ACK != 0 {
                    return;
                }
                (id, ModelState::SynSent)
            }
            Probe::Udp {
                source,
                destination,
            } => (FlowId::Udp(source, destination), ModelState::UdpOneWay),
            Probe::Echo {
                reply: false,
                identifier,
            } => (FlowId::Echo(identifier), ModelState::Echo),
            Probe::Echo { reply: true, .. } => return,
        };
        match self.find(local, remote, id, now) {
            Some(index) => self.touch(index, self.flows[index].3, now),
            None => self
                .flows
                .push((local, remote, id, initial, now + secs(initial.seconds()))),
        }
    }

    fn inbound(
        &mut self,
        policy: &CompiledFilter,
        remote: IpAddr,
        local: IpAddr,
        probe: Probe,
        now: Duration,
    ) -> bool {
        let tracked = match probe {
            Probe::Tcp {
                source,
                destination,
                flags,
            } => self
                .find(local, remote, FlowId::Tcp(destination, source), now)
                .map(|index| (index, model_tcp(self.flows[index].3, flags, false))),
            Probe::Udp {
                source,
                destination,
            } => self
                .find(local, remote, FlowId::Udp(destination, source), now)
                .map(|index| (index, ModelState::UdpTwoWay)),
            Probe::Echo {
                reply: true,
                identifier,
            } => self
                .find(local, remote, FlowId::Echo(identifier), now)
                .map(|index| (index, ModelState::Echo)),
            Probe::Echo { reply: false, .. } => None,
        };
        if let Some((index, state)) = tracked {
            self.touch(index, state, now);
            return true;
        }
        let (protocol, port) = match probe {
            Probe::Tcp { destination, .. } => (PacketProtocol::Tcp, Some(destination)),
            Probe::Udp { destination, .. } => (PacketProtocol::Udp, Some(destination)),
            Probe::Echo { .. } => (PacketProtocol::Icmp, None),
        };
        policy.allows(remote, protocol, port)
    }
}

#[derive(Debug, Clone)]
enum Step {
    Outbound {
        peer: u8,
        v6: bool,
        probe: Probe,
    },
    Inbound {
        peer: u8,
        v6: bool,
        probe: Probe,
    },
    /// An inbound answer to an earlier outbound packet, with fresh TCP flags.
    Reply {
        pick: prop::sample::Index,
        flags: u8,
    },
    Expire,
    SetPolicy(usize),
}

fn flags() -> impl Strategy<Value = u8> {
    prop::sample::select(vec![SYN, SYN_ACK, ACK, FIN, RST])
}

fn probe() -> impl Strategy<Value = Probe> {
    // Few ports and identifiers, so replies and collisions are common.
    let port = || prop::sample::select(vec![22_u16, 1000]);
    prop_oneof![
        (port(), port(), flags()).prop_map(|(source, destination, flags)| tcp(
            source,
            destination,
            flags
        )),
        (port(), port()).prop_map(|(source, destination)| Probe::Udp {
            source,
            destination
        }),
        (any::<bool>(), 1_u16..=2)
            .prop_map(|(reply, identifier)| Probe::Echo { reply, identifier }),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    let peer = 1..=PEERS;
    prop_oneof![
        8 => (peer.clone(), any::<bool>(), probe())
            .prop_map(|(peer, v6, probe)| Step::Outbound { peer, v6, probe }),
        4 => (peer, any::<bool>(), probe())
            .prop_map(|(peer, v6, probe)| Step::Inbound { peer, v6, probe }),
        8 => (any::<prop::sample::Index>(), flags())
            .prop_map(|(pick, flags)| Step::Reply { pick, flags }),
        1 => Just(Step::Expire),
        1 => (0..policies().len()).prop_map(Step::SetPolicy),
    ]
}

/// Mostly small steps, sometimes exactly a timeout, sometimes long gaps.
fn advance() -> impl Strategy<Value = Duration> {
    prop_oneof![
        12 => Just(Duration::ZERO),
        12 => (0_u64..5).prop_map(secs),
        2 => prop::sample::select(vec![10_u64, 30, 120]).prop_map(secs),
        1 => (0_u64..8000).prop_map(secs),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn inbound_verdicts_match_a_naive_reference_model(
        initial in 0..policies().len(),
        steps in prop::collection::vec((advance(), step()), 1..200),
    ) {
        let policies = policies().map(|policy| compiled(&policy));
        let mut policy = policies[initial].clone();
        // Room for every distinct flow the steps can open (3 peers × 2 families × 10
        // flows), in a table small enough that probe paths collide.
        let mut filter = Filter::with_capacity(policy.clone(), 60);
        let mut model = Model::default();
        let mut now = Duration::ZERO;
        let mut sent = Vec::new();
        let addresses = |peer, is_v6| if is_v6 { (v6(0), v6(peer)) } else { (v4(0), v4(peer)) };
        for (advance, step) in steps {
            now += advance;
            let (remote, local, probe) = match step {
                Step::Outbound { peer, v6: is_v6, probe } => {
                    let (local, remote) = addresses(peer, is_v6);
                    let verdict = filter.outbound(&packet(local, remote, probe), now);
                    prop_assert_eq!(verdict, Verdict::Accept);
                    model.outbound(local, remote, probe, now);
                    sent.push((remote, local, probe.reversed()));
                    continue;
                }
                Step::Inbound { peer, v6: is_v6, probe } => {
                    let (local, remote) = addresses(peer, is_v6);
                    (remote, local, probe)
                }
                Step::Reply { pick, flags } => {
                    let Some(&(remote, local, probe)) = sent.get(pick.index(sent.len().max(1)))
                    else {
                        continue;
                    };
                    let probe = match probe {
                        Probe::Tcp { source, destination, .. } => tcp(source, destination, flags),
                        Probe::Udp { .. } | Probe::Echo { .. } => probe,
                    };
                    (remote, local, probe)
                }
                Step::Expire => {
                    filter.expire(now);
                    model.purge(now);
                    prop_assert_eq!(filter.len(), model.flows.len());
                    continue;
                }
                Step::SetPolicy(index) => {
                    policy = policies[index].clone();
                    filter.set_policy(policy.clone());
                    continue;
                }
            };
            let verdict = filter.inbound(&packet(remote, local, probe), now);
            let expected = model.inbound(&policy, remote, local, probe, now);
            prop_assert_eq!(verdict == Verdict::Accept, expected, "{:?} at {:?}", probe, now);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic_in_either_direction(
        random in prop::collection::vec(any::<u8>(), 0..96),
        seed in (any::<bool>(), any::<bool>(), probe()),
        edits in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..6),
        cut in any::<prop::sample::Index>(),
    ) {
        // Mutated and truncated valid packets reach deeper than pure noise.
        let (is_v6, outbound, probe) = seed;
        let (local, remote) = if is_v6 { (v6(0), v6(1)) } else { (v4(0), v4(1)) };
        let (source, destination) = if outbound { (local, remote) } else { (remote, local) };
        let mut mutated = packet(source, destination, probe);
        for (index, byte) in edits {
            let index = index.index(mutated.len());
            mutated[index] = byte;
        }
        mutated.truncate(cut.index(mutated.len() + 1));

        let mut filter = Filter::with_capacity(compiled(&Policy::default_allow_all()), 2);
        for (step, bytes) in [random, mutated].iter().enumerate() {
            let now = secs(u64::try_from(step).unwrap());
            filter.outbound(bytes, now);
            filter.inbound(bytes, now);
        }
    }
}
