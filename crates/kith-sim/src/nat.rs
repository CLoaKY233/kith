//! NAT behaviour in the terms of RFC 4787: how mappings are chosen and how inbound
//! packets are filtered.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// How a NAT picks the public port for an outbound flow (RFC 4787 section 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    /// One public port per internal endpoint, whatever the destination ("easy").
    EndpointIndependent,
    /// A new public port per destination ("hard", symmetric).
    EndpointDependent,
}

/// Which inbound packets a NAT lets through a mapping (RFC 4787 section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filtering {
    /// Anything sent to the public port.
    EndpointIndependent,
    /// Only packets from an address and port the mapping has sent to.
    AddressAndPortDependent,
}

/// A NAT's behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatConfig {
    /// How public ports are chosen.
    pub mapping: Mapping,
    /// Which inbound packets pass.
    pub filtering: Filtering,
    /// How long a mapping lives after its last outbound packet.
    pub timeout: Duration,
}

impl NatConfig {
    /// A port-restricted cone NAT: hole punching works.
    pub const EASY: Self = Self {
        mapping: Mapping::EndpointIndependent,
        filtering: Filtering::AddressAndPortDependent,
        timeout: Duration::from_secs(30),
    };

    /// A symmetric NAT: hole punching against another one fails.
    pub const HARD: Self = Self {
        mapping: Mapping::EndpointDependent,
        filtering: Filtering::AddressAndPortDependent,
        timeout: Duration::from_secs(30),
    };
}

#[derive(Debug)]
struct Binding {
    internal: SocketAddr,
    key: (SocketAddr, Option<SocketAddr>),
    expires: Duration,
    contacted: BTreeSet<SocketAddr>,
}

#[derive(Debug)]
pub(crate) struct Nat {
    config: NatConfig,
    public: IpAddr,
    next_port: u16,
    mappings: BTreeMap<(SocketAddr, Option<SocketAddr>), u16>,
    bindings: BTreeMap<u16, Binding>,
}

impl Nat {
    pub(crate) const fn new(config: NatConfig, public: IpAddr) -> Self {
        Self {
            config,
            public,
            next_port: 40_000,
            mappings: BTreeMap::new(),
            bindings: BTreeMap::new(),
        }
    }

    pub(crate) const fn public(&self) -> IpAddr {
        self.public
    }

    /// Translates an outbound packet's source, creating or refreshing its mapping.
    pub(crate) fn outbound(
        &mut self,
        now: Duration,
        internal: SocketAddr,
        remote: SocketAddr,
    ) -> SocketAddr {
        self.expire(now);
        let key = match self.config.mapping {
            Mapping::EndpointIndependent => (internal, None),
            Mapping::EndpointDependent => (internal, Some(remote)),
        };
        let port = if let Some(port) = self.mappings.get(&key) {
            *port
        } else {
            let port = self.allocate_port();
            self.mappings.insert(key, port);
            self.bindings.insert(
                port,
                Binding {
                    internal,
                    key,
                    expires: now,
                    contacted: BTreeSet::new(),
                },
            );
            port
        };
        if let Some(binding) = self.bindings.get_mut(&port) {
            binding.expires = now.saturating_add(self.config.timeout);
            binding.contacted.insert(remote);
        }
        SocketAddr::new(self.public, port)
    }

    /// The internal endpoint an inbound packet from `remote` to `port` reaches, if any.
    pub(crate) fn inbound(
        &mut self,
        now: Duration,
        remote: SocketAddr,
        port: u16,
    ) -> Option<SocketAddr> {
        self.expire(now);
        let binding = self.bindings.get(&port)?;
        match self.config.filtering {
            Filtering::EndpointIndependent => Some(binding.internal),
            Filtering::AddressAndPortDependent => binding
                .contacted
                .contains(&remote)
                .then_some(binding.internal),
        }
    }

    fn allocate_port(&mut self) -> u16 {
        while self.bindings.contains_key(&self.next_port) {
            self.next_port = self.next_port.checked_add(1).unwrap_or(1024);
        }
        let port = self.next_port;
        self.next_port = self.next_port.checked_add(1).unwrap_or(1024);
        port
    }

    fn expire(&mut self, now: Duration) {
        let expired: Vec<u16> = self
            .bindings
            .iter()
            .filter(|(_, binding)| binding.expires <= now)
            .map(|(port, _)| *port)
            .collect();
        for port in expired {
            if let Some(binding) = self.bindings.remove(&port) {
                self.mappings.remove(&binding.key);
            }
        }
    }
}
