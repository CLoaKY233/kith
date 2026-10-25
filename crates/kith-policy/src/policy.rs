//! The policy model and its wire encoding.
//!
//! A policy is a list of allow rules and nothing else. There are no deny rules and no rule
//! order, so a policy means the same thing however its rules are arranged, and adding a rule
//! can only open access, never close it. Anything no rule allows is denied.

use alloc::vec;
use alloc::vec::Vec;

use kith_types::NodeId;
use kith_wire::{MessageType, WireError, envelope, registry};
use serde::{Deserialize, Serialize};

use crate::ports::PortSet;
use crate::tag::{TagError, TagName};

/// The most rules one policy may hold.
pub const MAX_RULES: usize = 256;

/// The most source selectors, and separately the most destination targets, in one rule.
pub const MAX_SELECTORS_PER_RULE: usize = 64;

/// The envelope message type of an encoded [`Policy`], from [`registry::POLICY_DOCUMENT`].
pub const POLICY_MESSAGE_TYPE: MessageType = registry::POLICY_DOCUMENT;

/// The envelope version this crate writes and the newest it reads.
pub const POLICY_VERSION: u64 = 1;

/// Why a policy, or a part of one, was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// More than [`MAX_RULES`] rules.
    #[error("{count} rules, more than {MAX_RULES}")]
    TooManyRules {
        /// The number of rules given.
        count: usize,
    },
    /// A rule had no sources, so it could never match.
    #[error("rule has no sources")]
    EmptySources,
    /// A rule had no destinations, so it could never match.
    #[error("rule has no destinations")]
    EmptyDestinations,
    /// A rule had more than [`MAX_SELECTORS_PER_RULE`] sources or destinations.
    #[error("{count} selectors in one rule list, more than {MAX_SELECTORS_PER_RULE}")]
    TooManySelectors {
        /// The number of selectors given.
        count: usize,
    },
    /// A port range ended before it started.
    #[error("port range {start}-{end} ends before it starts")]
    InvalidPortRange {
        /// The first port given.
        start: u16,
        /// The last port given.
        end: u16,
    },
    /// A port list was empty. Use [`PortSet::All`] for every port.
    #[error("port list is empty")]
    EmptyPortRanges,
    /// More than [`MAX_PORT_RANGES`](crate::MAX_PORT_RANGES) ranges after merging.
    #[error("{count} port ranges, more than {}", crate::MAX_PORT_RANGES)]
    TooManyPortRanges {
        /// The number of ranges.
        count: usize,
    },
    /// Encoded port ranges were not sorted and merged.
    #[error("encoded port ranges are not normalized")]
    PortRangesNotNormalized,
    /// Port ranges were given for ICMP, which has no ports.
    #[error("ICMP has no ports; use all ports")]
    PortsWithoutPortProtocol,
    /// A tag name was invalid.
    #[error("invalid tag: {0}")]
    Tag(#[from] TagError),
}

/// Which devices a selector names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Selector {
    /// Every member of the network.
    Any,
    /// One device.
    Device(NodeId),
    /// Every member carrying the tag.
    Tag(TagName),
}

/// The transport protocol a target opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Protocol {
    /// Every IP protocol.
    Any,
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
    /// ICMP for IPv4, `ICMPv6` for IPv6.
    Icmp,
}

/// The protocol of a concrete packet, as seen by the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PacketProtocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
    /// ICMP or `ICMPv6`.
    Icmp,
    /// Any other IP protocol, by number. Only [`Protocol::Any`] matches it.
    Other(u8),
}

impl Protocol {
    /// Whether a packet of `protocol` is covered.
    #[must_use]
    pub const fn matches(self, protocol: PacketProtocol) -> bool {
        matches!(
            (self, protocol),
            (Self::Any, _)
                | (Self::Tcp, PacketProtocol::Tcp)
                | (Self::Udp, PacketProtocol::Udp)
                | (Self::Icmp, PacketProtocol::Icmp)
        )
    }
}

/// One destination of a rule: which devices, and which traffic to them.
///
/// Invariant: ICMP targets open all ports, because ICMP has none.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawTarget")]
pub struct Target {
    selector: Selector,
    protocol: Protocol,
    ports: PortSet,
}

impl Target {
    /// A target. Fails if `protocol` is ICMP and `ports` is not [`PortSet::All`].
    pub fn new(
        selector: Selector,
        protocol: Protocol,
        ports: PortSet,
    ) -> Result<Self, PolicyError> {
        if protocol == Protocol::Icmp && ports != PortSet::All {
            return Err(PolicyError::PortsWithoutPortProtocol);
        }
        Ok(Self {
            selector,
            protocol,
            ports,
        })
    }

    /// All traffic to `selector`.
    #[must_use]
    pub const fn all_traffic(selector: Selector) -> Self {
        Self {
            selector,
            protocol: Protocol::Any,
            ports: PortSet::All,
        }
    }

    /// The devices this target names.
    #[must_use]
    pub const fn selector(&self) -> &Selector {
        &self.selector
    }

    /// The protocol opened.
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// The destination ports opened.
    #[must_use]
    pub const fn ports(&self) -> &PortSet {
        &self.ports
    }
}

#[derive(Deserialize)]
struct RawTarget {
    selector: Selector,
    protocol: Protocol,
    ports: PortSet,
}

impl TryFrom<RawTarget> for Target {
    type Error = PolicyError;

    fn try_from(raw: RawTarget) -> Result<Self, Self::Error> {
        Self::new(raw.selector, raw.protocol, raw.ports)
    }
}

/// One allow rule: any device matching a source may reach any destination target.
///
/// Invariant: 1 to [`MAX_SELECTORS_PER_RULE`] sources and destinations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawRule")]
pub struct Rule {
    sources: Vec<Selector>,
    destinations: Vec<Target>,
}

impl Rule {
    /// A rule. Fails if either list is empty or longer than [`MAX_SELECTORS_PER_RULE`].
    pub fn new(sources: Vec<Selector>, destinations: Vec<Target>) -> Result<Self, PolicyError> {
        check_len(sources.len(), PolicyError::EmptySources)?;
        check_len(destinations.len(), PolicyError::EmptyDestinations)?;
        Ok(Self {
            sources,
            destinations,
        })
    }

    /// Which devices may send.
    #[must_use]
    pub fn sources(&self) -> &[Selector] {
        &self.sources
    }

    /// What they may reach.
    #[must_use]
    pub fn destinations(&self) -> &[Target] {
        &self.destinations
    }
}

fn check_len(count: usize, empty: PolicyError) -> Result<(), PolicyError> {
    if count == 0 {
        Err(empty)
    } else if count > MAX_SELECTORS_PER_RULE {
        Err(PolicyError::TooManySelectors { count })
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
struct RawRule {
    sources: Vec<Selector>,
    destinations: Vec<Target>,
}

impl TryFrom<RawRule> for Rule {
    type Error = PolicyError;

    fn try_from(raw: RawRule) -> Result<Self, Self::Error> {
        Self::new(raw.sources, raw.destinations)
    }
}

/// A network's access policy: a default-deny list of allow rules.
///
/// Invariant: at most [`MAX_RULES`] rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "RawPolicy")]
pub struct Policy {
    rules: Vec<Rule>,
}

impl Policy {
    /// A policy of `rules`. Fails if there are more than [`MAX_RULES`].
    pub fn new(rules: Vec<Rule>) -> Result<Self, PolicyError> {
        if rules.len() > MAX_RULES {
            return Err(PolicyError::TooManyRules { count: rules.len() });
        }
        Ok(Self { rules })
    }

    /// The policy that denies everything. Same as [`Policy::default`].
    #[must_use]
    pub const fn deny_all() -> Self {
        Self { rules: Vec::new() }
    }

    /// The policy for a brand-new network: every member may reach every other on every port.
    #[must_use]
    pub fn default_allow_all() -> Self {
        Self {
            rules: vec![Rule {
                sources: vec![Selector::Any],
                destinations: vec![Target::all_traffic(Selector::Any)],
            }],
        }
    }

    /// The rules.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Encodes the policy in a [`POLICY_MESSAGE_TYPE`] envelope, version [`POLICY_VERSION`].
    ///
    /// The encoding is canonical: [`Policy::from_bytes`] accepts exactly these bytes back.
    pub fn to_bytes(&self) -> Result<Vec<u8>, WireError> {
        envelope::encode(POLICY_MESSAGE_TYPE, POLICY_VERSION, self)
    }

    /// Decodes and fully re-validates a policy written by [`Policy::to_bytes`].
    ///
    /// Rejects the wrong type or version, trailing bytes, and any broken invariant.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        let (version, policy) = envelope::decode(bytes, POLICY_MESSAGE_TYPE, POLICY_VERSION)?;
        if version == POLICY_VERSION {
            Ok(policy)
        } else {
            Err(WireError::UnsupportedVersion {
                found: version,
                supported: POLICY_VERSION,
            })
        }
    }
}

#[derive(Deserialize)]
struct RawPolicy {
    rules: Vec<Rule>,
}

impl TryFrom<RawPolicy> for Policy {
    type Error = PolicyError;

    fn try_from(raw: RawPolicy) -> Result<Self, Self::Error> {
        Self::new(raw.rules)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec;
    use alloc::vec::Vec;

    use kith_types::NodeId;
    use kith_wire::{WireError, envelope};
    use proptest::prelude::*;

    use super::{
        MAX_RULES, MAX_SELECTORS_PER_RULE, POLICY_MESSAGE_TYPE, PacketProtocol, Policy,
        PolicyError, Protocol, Rule, Selector, Target,
    };
    use crate::{PortRange, PortSet, TagName};

    fn sample() -> Policy {
        let ssh = Target::new(
            Selector::Tag(TagName::new("srv").unwrap()),
            Protocol::Tcp,
            PortSet::ranges(vec![PortRange::single(22), PortRange::new(80, 81).unwrap()]).unwrap(),
        )
        .unwrap();
        let rule = Rule::new(
            vec![
                Selector::Device(NodeId::from_bytes([0xab; 32])),
                Selector::Any,
            ],
            vec![
                ssh,
                Target::new(Selector::Any, Protocol::Icmp, PortSet::All).unwrap(),
            ],
        )
        .unwrap();
        Policy::new(vec![rule]).unwrap()
    }

    #[test]
    fn golden_bytes() {
        // Pins the format: changing these bytes is a protocol break.
        let mut expected = vec![b'k', b't', 0x30, 1, 1, 2, 1];
        expected.extend_from_slice(&[0xab; 32]);
        expected.extend_from_slice(&[
            0, 2, 2, 3, b's', b'r', b'v', 1, 1, 2, 22, 22, 80, 81, 0, 3, 0,
        ]);
        assert_eq!(sample().to_bytes().unwrap(), expected);
        assert_eq!(Policy::from_bytes(&expected), Ok(sample()));
        assert_eq!(
            Policy::default_allow_all().to_bytes().unwrap(),
            [b'k', b't', 0x30, 1, 1, 1, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            Policy::deny_all().to_bytes().unwrap(),
            [b'k', b't', 0x30, 1, 0]
        );
    }

    #[test]
    fn decoding_revalidates() {
        let bytes = sample().to_bytes().unwrap();
        let mut bad_tag = bytes.clone();
        let tag_at = bad_tag.iter().position(|&byte| byte == b's').unwrap();
        bad_tag[tag_at] = b'S';
        assert_eq!(Policy::from_bytes(&bad_tag), Err(WireError::Decode));
        // Reversed port range 81..80.
        let mut bad_range = bytes.clone();
        let len = bad_range.len();
        bad_range[len - 5] = 81;
        bad_range[len - 4] = 80;
        assert_eq!(Policy::from_bytes(&bad_range), Err(WireError::Decode));
        // Unnormalized (adjacent) port ranges 22..22, 23..23.
        let mut adjacent = bytes.clone();
        adjacent[len - 5] = 23;
        adjacent[len - 4] = 23;
        assert_eq!(Policy::from_bytes(&adjacent), Err(WireError::Decode));
        // ICMP with ports: protocol byte of the ssh target becomes ICMP (3).
        let mut icmp_ports = bytes.clone();
        icmp_ports[len - 10] = 3;
        assert_eq!(Policy::from_bytes(&icmp_ports), Err(WireError::Decode));
        let empty_sources = [b'k', b't', 0x30, 1, 1, 0, 1, 0, 0, 0];
        assert_eq!(Policy::from_bytes(&empty_sources), Err(WireError::Decode));
        let v2 = envelope::encode(POLICY_MESSAGE_TYPE, 2, &Policy::deny_all()).unwrap();
        assert!(matches!(
            Policy::from_bytes(&v2),
            Err(WireError::UnsupportedVersion { .. })
        ));
        let other = envelope::encode(kith_wire::MessageType(1), 1, &Policy::deny_all()).unwrap();
        assert!(matches!(
            Policy::from_bytes(&other),
            Err(WireError::UnexpectedType { .. })
        ));
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(Policy::from_bytes(&trailing), Err(WireError::TrailingBytes));
    }

    #[test]
    fn decoding_enforces_rule_limit() {
        // Encode through a plain struct, bypassing the constructor's check.
        #[derive(serde::Serialize)]
        struct Unchecked {
            rules: Vec<Rule>,
        }
        let rule = Policy::default_allow_all().rules()[0].clone();
        let rules: Vec<Rule> = vec![rule; MAX_RULES + 1];
        let bytes = envelope::encode(POLICY_MESSAGE_TYPE, 1, &Unchecked { rules }).unwrap();
        assert_eq!(Policy::from_bytes(&bytes), Err(WireError::Decode));
    }

    #[test]
    fn constructor_validation() {
        let any = Target::all_traffic(Selector::Any);
        assert_eq!(
            Rule::new(vec![], vec![any.clone()]),
            Err(PolicyError::EmptySources)
        );
        assert_eq!(
            Rule::new(vec![Selector::Any], vec![]),
            Err(PolicyError::EmptyDestinations)
        );
        assert_eq!(
            Rule::new(
                vec![Selector::Any; MAX_SELECTORS_PER_RULE + 1],
                vec![any.clone()]
            ),
            Err(PolicyError::TooManySelectors {
                count: MAX_SELECTORS_PER_RULE + 1
            })
        );
        assert_eq!(
            Rule::new(vec![Selector::Any], vec![any; MAX_SELECTORS_PER_RULE + 1]),
            Err(PolicyError::TooManySelectors {
                count: MAX_SELECTORS_PER_RULE + 1
            })
        );
        let rule = Policy::default_allow_all().rules()[0].clone();
        assert_eq!(
            Policy::new(vec![rule; MAX_RULES + 1]),
            Err(PolicyError::TooManyRules {
                count: MAX_RULES + 1
            })
        );
        assert_eq!(
            Target::new(Selector::Any, Protocol::Icmp, PortSet::single(1)),
            Err(PolicyError::PortsWithoutPortProtocol)
        );
    }

    #[test]
    fn protocol_matching() {
        use PacketProtocol::{Icmp, Other, Tcp, Udp};
        assert!(
            [Tcp, Udp, Icmp, Other(47)]
                .into_iter()
                .all(|p| Protocol::Any.matches(p))
        );
        assert!(Protocol::Tcp.matches(Tcp) && !Protocol::Tcp.matches(Udp));
        assert!(Protocol::Udp.matches(Udp) && !Protocol::Udp.matches(Icmp));
        assert!(Protocol::Icmp.matches(Icmp) && !Protocol::Icmp.matches(Other(1)));
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(input in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ignored = Policy::from_bytes(&input);
        }

        #[test]
        fn arbitrary_bodies_never_panic(body in proptest::collection::vec(any::<u8>(), 0..512)) {
            let mut input = vec![b'k', b't', 0x30, 1];
            input.extend_from_slice(&body);
            if let Ok(policy) = Policy::from_bytes(&input) {
                // Whatever decodes re-encodes to the same bytes: the encoding is canonical.
                prop_assert_eq!(policy.to_bytes().unwrap(), input);
            }
        }
    }
}
