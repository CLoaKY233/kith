//! Well-known ledger record keys and their typed values.
//!
//! Unknown keys are ignored, so newer software can add keys without breaking older devices.
//!
//! - [`POLICY`]: a [`Policy::to_bytes`](kith_policy::Policy::to_bytes) encoding. Absent
//!   means [`Policy::default_allow_all`](kith_policy::Policy::default_allow_all).
//!   Undecodable means deny-all (except the device's own traffic) plus a warning: failing
//!   open would silently undo a policy meant to restrict traffic.
//! - [`Record`]s under `route/<prefix>`, `relay/<name>`, `dns/<name>` and `exit/<node hex>`.
//!   Each value is a [`CONTROL_RECORD`](kith_wire::registry::CONTROL_RECORD) envelope whose
//!   postcard body starts with a kind tag, which must match the key's prefix.
//!
//! Keys must be canonical (a canonical prefix, lowercase hex, lowercase names without a
//! trailing dot) so that each route, relay, DNS name and device has exactly one key.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use kith_ledger::MAX_RECORD_KEY_LEN;
use kith_types::{IpPrefix, NodeId};
use kith_wire::{WireError, envelope, registry};
use serde::{Deserialize, Serialize};

/// The key holding the encoded network policy.
pub const POLICY: &str = "policy";

/// The prefix of subnet route advertisements: `route/<prefix>`.
pub const ROUTE_PREFIX: &str = "route/";

/// The prefix of relay server advertisements: `relay/<name>`.
pub const RELAY_PREFIX: &str = "relay/";

/// The prefix of custom DNS records: `dns/<name>`.
pub const DNS_PREFIX: &str = "dns/";

/// The prefix of exit node grants: `exit/<node hex>`.
pub const EXIT_PREFIX: &str = "exit/";

const RECORD_VERSION: u64 = 1;

const MAX_TTL: u32 = 0x7FFF_FFFF;

const MAX_DNS_RECORDS: usize = 64;

/// Why a typed record is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecordError {
    /// The value is not a record envelope this build understands.
    #[error("undecodable record value: {0}")]
    Value(#[from] WireError),
    /// The value is a different kind of record than its key.
    #[error("record value is a different kind than its key")]
    KindMismatch,
    /// The key after its prefix is malformed or not canonical.
    #[error("record key is malformed or not canonical")]
    InvalidKey,
    /// A relay address is not `host:port` with a nonzero port.
    #[error("relay address is not host:port")]
    InvalidAddress,
    /// A DNS record set is empty or holds more than 64 records.
    #[error("a DNS name needs 1 to {MAX_DNS_RECORDS} records")]
    DnsRecordCount,
    /// A CNAME shares its name with another record (RFC 1034 §3.6.2).
    #[error("a CNAME must be the only record at its name")]
    CnameNotAlone,
    /// A TTL above 2³¹ − 1 seconds.
    #[error("TTL {0} exceeds {MAX_TTL}")]
    TtlTooLarge(u32),
}

/// A typed ledger record: its key and decoded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// `route/<prefix>`: `via` forwards traffic for a subnet.
    Route {
        /// The advertised subnet.
        prefix: IpPrefix,
        /// The advertising device.
        via: NodeId,
    },
    /// `relay/<name>`: a relay server.
    Relay(Relay),
    /// `dns/<name>`: custom DNS records.
    Dns(DnsRecords),
    /// `exit/<node hex>`: the device may act as an exit node.
    Exit(NodeId),
}

impl Record {
    /// The ledger key.
    pub fn key(&self) -> String {
        match self {
            Self::Route { prefix, .. } => format!("{ROUTE_PREFIX}{prefix}"),
            Self::Relay(relay) => format!("{RELAY_PREFIX}{}", relay.name),
            Self::Dns(records) => format!("{DNS_PREFIX}{}", records.name),
            Self::Exit(node) => format!("{EXIT_PREFIX}{node}"),
        }
    }

    /// Encodes the ledger value.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let body = match self {
            Self::Route { via, .. } => Body::Route(*via),
            Self::Relay(relay) => Body::Relay {
                address: relay.address.clone(),
                node: relay.node,
            },
            Self::Dns(records) => Body::Dns {
                ttl: records.ttl,
                data: records.data.clone(),
            },
            Self::Exit(_) => Body::Exit,
        };
        envelope::encode(registry::CONTROL_RECORD, RECORD_VERSION, &body)
    }

    /// Decodes a ledger entry, or returns `None` if `key` is not a typed record key.
    pub fn decode(key: &str, value: &[u8]) -> Option<Result<Self, RecordError>> {
        let (kind, name) = [
            (Kind::Route, ROUTE_PREFIX),
            (Kind::Relay, RELAY_PREFIX),
            (Kind::Dns, DNS_PREFIX),
            (Kind::Exit, EXIT_PREFIX),
        ]
        .into_iter()
        .find_map(|(kind, prefix)| Some((kind, key.strip_prefix(prefix)?)))?;
        let record = Self::decode_value(kind, name, value).and_then(|record| {
            (record.key() == key)
                .then_some(record)
                .ok_or(RecordError::InvalidKey)
        });
        Some(record)
    }

    fn decode_value(kind: Kind, name: &str, value: &[u8]) -> Result<Self, RecordError> {
        let (version, body) =
            envelope::decode::<Body>(value, registry::CONTROL_RECORD, RECORD_VERSION)?;
        if version != RECORD_VERSION {
            return Err(WireError::UnsupportedVersion {
                found: version,
                supported: RECORD_VERSION,
            }
            .into());
        }
        if body.kind() != kind {
            return Err(RecordError::KindMismatch);
        }
        match body {
            Body::Route(via) => {
                let prefix = name.parse().map_err(|_| RecordError::InvalidKey)?;
                Ok(Self::Route { prefix, via })
            }
            Body::Relay { address, node } => {
                Relay::new(String::from(name), address, node).map(Self::Relay)
            }
            Body::Dns { ttl, data } => {
                DnsRecords::new(String::from(name), ttl, data).map(Self::Dns)
            }
            Body::Exit => name
                .parse()
                .map(Self::Exit)
                .map_err(|_| RecordError::InvalidKey),
        }
    }

    pub(crate) const fn device(&self) -> Option<NodeId> {
        match self {
            Self::Route { via, .. } => Some(*via),
            Self::Relay(relay) => Some(relay.node),
            Self::Exit(node) => Some(*node),
            Self::Dns(_) => None,
        }
    }
}

/// A relay server advertised under `relay/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    name: String,
    address: String,
    node: NodeId,
}

impl Relay {
    /// Validates a relay: a canonical name and a `host:port` address (IPv6 in brackets).
    pub fn new(name: String, address: String, node: NodeId) -> Result<Self, RecordError> {
        check_name(RELAY_PREFIX, &name)?;
        if !is_host_port(&address) {
            return Err(RecordError::InvalidAddress);
        }
        Ok(Self {
            name,
            address,
            node,
        })
    }

    /// The name in the record key.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where to reach the relay: `host:port`.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The relay server's identity, which its TLS key must match.
    pub const fn node(&self) -> NodeId {
        self.node
    }
}

/// Custom DNS records at one name, sharing one TTL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsRecords {
    name: String,
    ttl: u32,
    data: Vec<DnsData>,
}

impl DnsRecords {
    /// Validates the records: a canonical name, 1 to 64 records, a CNAME only on its own
    /// and a 31-bit TTL. Names and text are checked by the naming plane.
    pub fn new(name: String, ttl: u32, data: Vec<DnsData>) -> Result<Self, RecordError> {
        check_name(DNS_PREFIX, &name)?;
        if data.is_empty() || data.len() > MAX_DNS_RECORDS {
            return Err(RecordError::DnsRecordCount);
        }
        if data.len() > 1 && data.iter().any(|data| matches!(data, DnsData::Cname(_))) {
            return Err(RecordError::CnameNotAlone);
        }
        if ttl > MAX_TTL {
            return Err(RecordError::TtlTooLarge(ttl));
        }
        Ok(Self { name, ttl, data })
    }

    /// The owner name in the record key.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The TTL in seconds.
    pub const fn ttl(&self) -> u32 {
        self.ttl
    }

    /// The records, in the order written.
    pub fn data(&self) -> &[DnsData] {
        &self.data
    }
}

/// One custom DNS record's data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsData {
    /// IPv4 address.
    A(Ipv4Addr),
    /// IPv6 address.
    Aaaa(Ipv6Addr),
    /// Alias to another name.
    Cname(String),
    /// Text, as its character-strings.
    Txt(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Route,
    Relay,
    Dns,
    Exit,
}

#[derive(Serialize, Deserialize)]
enum Body {
    Route(NodeId),
    Relay { address: String, node: NodeId },
    Dns { ttl: u32, data: Vec<DnsData> },
    Exit,
}

impl Body {
    const fn kind(&self) -> Kind {
        match self {
            Self::Route(_) => Kind::Route,
            Self::Relay { .. } => Kind::Relay,
            Self::Dns { .. } => Kind::Dns,
            Self::Exit => Kind::Exit,
        }
    }
}

fn check_name(prefix: &str, name: &str) -> Result<(), RecordError> {
    let valid = !name.is_empty()
        && prefix.len().saturating_add(name.len()) <= MAX_RECORD_KEY_LEN
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_uppercase());
    valid.then_some(()).ok_or(RecordError::InvalidKey)
}

fn is_host_port(address: &str) -> bool {
    let Some((host, port)) = address.rsplit_once(':') else {
        return false;
    };
    let port_valid = port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0);
    let host_valid = match host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
    {
        Some(ipv6) => ipv6.parse::<Ipv6Addr>().is_ok(),
        None => {
            !host.is_empty()
                && host
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b':' | b'[' | b']'))
        }
    };
    port_valid && host_valid
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::string::{String, ToString};
    use alloc::vec;
    use core::net::{Ipv4Addr, Ipv6Addr};

    use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NodeId};
    use proptest::prelude::*;

    use super::{DnsData, DnsRecords, MAX_TTL, Record, RecordError, Relay};

    const NODE: NodeId = NodeId::from_bytes([7; 32]);

    #[test]
    fn record_values_have_golden_bytes() {
        let header = [b'k', b't', 0x62, 1];
        let golden = |record: Record, body: &[u8]| {
            assert_eq!(
                record.encode().unwrap(),
                [&header, body].concat(),
                "{record:?}"
            );
        };
        let route = Record::Route {
            prefix: "10.1.0.0/16".parse().unwrap(),
            via: NODE,
        };
        golden(route, &[[0].as_slice(), &[7; 32]].concat());
        let relay = Relay::new("eu".into(), "relay.example.com:443".into(), NODE).unwrap();
        let address = b"relay.example.com:443";
        golden(
            Record::Relay(relay),
            &[&[1, 21], &address[..], &[7; 32]].concat(),
        );
        let data = vec![
            DnsData::A(Ipv4Addr::new(100, 64, 0, 5)),
            DnsData::Txt(vec!["hi".into()]),
        ];
        let dns = DnsRecords::new("nas".into(), 300, data).unwrap();
        golden(
            Record::Dns(dns),
            &[2, 0xac, 0x02, 2, 0, 100, 64, 0, 5, 3, 1, 2, b'h', b'i'],
        );
        golden(Record::Exit(NODE), &[3]);

        let alias = DnsData::Cname("nas.home".into());
        let shared = DnsRecords::new(
            "www".into(),
            60,
            vec![alias, DnsData::A(Ipv4Addr::LOCALHOST)],
        );
        assert_eq!(shared, Err(RecordError::CnameNotAlone));
    }

    fn text(
        alphabet: &'static [u8],
        len: core::ops::Range<usize>,
    ) -> impl Strategy<Value = String> {
        proptest::collection::vec(proptest::sample::select(alphabet), len)
            .prop_map(|bytes| bytes.into_iter().map(char::from).collect())
    }

    fn name() -> impl Strategy<Value = String> {
        text(b"az09-_.*", 0..20).prop_map(|name| alloc::format!("{name}x"))
    }

    fn dns_data() -> impl Strategy<Value = DnsData> {
        prop_oneof![
            any::<u32>().prop_map(|addr| DnsData::A(Ipv4Addr::from(addr))),
            any::<u128>().prop_map(|addr| DnsData::Aaaa(Ipv6Addr::from(addr))),
            proptest::collection::vec(text(b"a =;\"", 0..8), 0..3).prop_map(DnsData::Txt),
        ]
    }

    fn record() -> impl Strategy<Value = Record> {
        let node = any::<[u8; 32]>().prop_map(NodeId::from_bytes);
        let host = prop_oneof![
            name(),
            any::<u32>().prop_map(|addr| Ipv4Addr::from(addr).to_string()),
            any::<u128>().prop_map(|addr| alloc::format!("[{}]", Ipv6Addr::from(addr))),
        ];
        let data = prop_oneof![
            proptest::collection::vec(dns_data(), 1..5),
            name().prop_map(|target| vec![DnsData::Cname(target)]),
        ];
        prop_oneof![
            (any::<u32>(), 0_u8..=32, node.clone()).prop_map(|(addr, len, via)| Record::Route {
                prefix: IpPrefix::V4(Ipv4Prefix::truncating(Ipv4Addr::from(addr), len).unwrap()),
                via,
            }),
            (any::<u128>(), 0_u8..=128, node.clone()).prop_map(|(addr, len, via)| Record::Route {
                prefix: IpPrefix::V6(Ipv6Prefix::truncating(Ipv6Addr::from(addr), len).unwrap()),
                via,
            }),
            (name(), host, 1_u16.., node.clone()).prop_map(|(name, host, port, node)| {
                Record::Relay(Relay::new(name, alloc::format!("{host}:{port}"), node).unwrap())
            }),
            (name(), 0..=MAX_TTL, data).prop_map(|(name, ttl, data)| {
                Record::Dns(DnsRecords::new(name, ttl, data).unwrap())
            }),
            node.prop_map(Record::Exit),
        ]
    }

    proptest! {
        #[test]
        fn records_round_trip_canonically_and_garbage_never_panics(
            record in record(),
            garbage in proptest::collection::vec(any::<u8>(), 0..64),
            suffix in text(b"0123456789abcdefABCDEF.:/[]", 0..70),
        ) {
            let key = record.key();
            let value = record.encode().unwrap();
            prop_assert_eq!(Record::decode(&key, &value), Some(Ok(record)));

            let (kind, rest) = key.split_once('/').unwrap();
            let upper = alloc::format!("{kind}/{}", rest.to_ascii_uppercase());
            if upper != key {
                prop_assert!(matches!(Record::decode(&upper, &value), Some(Err(_))));
            }

            let _ignored = Record::decode(&key, &[&value[..4], &garbage].concat());
            for prefix in [super::ROUTE_PREFIX, super::EXIT_PREFIX] {
                let _ignored = Record::decode(&alloc::format!("{prefix}{suffix}"), &value);
            }
        }
    }
}
