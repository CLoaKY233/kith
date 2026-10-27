use core::net::IpAddr;

use kith_types::IpPrefix;

use crate::{Domain, Interface};

/// Everything kith installs into the OS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The TUN interface.
    pub interface: Interface,
    /// Prefixes routed into the TUN.
    pub routes: Vec<IpPrefix>,
    /// Route all traffic through the TUN.
    pub exit_node: Option<ExitNode>,
    /// DNS configuration.
    pub dns: Option<Dns>,
    /// Enable IP forwarding, to act as an exit node or subnet router.
    pub forwarding: bool,
    /// Masquerade mesh traffic leaving through the default interface.
    pub nat: Option<Nat>,
}

impl Plan {
    /// A plan that installs nothing on `interface`.
    pub fn new(interface: Interface) -> Self {
        Self {
            interface,
            routes: Vec::new(),
            exit_node: None,
            dns: None,
            forwarding: false,
            nat: None,
        }
    }
}

impl Plan {
    /// Whether `new` leaves out something this plan installed, which applying `new` on top
    /// would keep.
    pub(crate) fn drops_into(&self, new: &Self) -> bool {
        self.interface != new.interface
            || (self.exit_node.is_some() && new.exit_node.is_none())
            || self.dns.as_ref().is_some_and(|dns| {
                new.dns.as_ref().is_none_or(|next| {
                    (dns.all && !next.all) || dns.domains.iter().any(|d| !next.domains.contains(d))
                })
            })
            || (self.nat.is_some() && new.nat.is_none())
            || (self.forwarding && !new.forwarding)
            || self.routes.iter().any(|route| !new.routes.contains(route))
    }
}

/// Exit-node client mode.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExitNode {
    /// Peer UDP endpoints that must keep using the original gateway (macOS).
    pub peer_endpoints: Vec<IpAddr>,
}

/// Mesh DNS configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dns {
    /// The mesh resolver.
    pub resolver: IpAddr,
    /// Domains answered by the mesh resolver.
    pub domains: Vec<Domain>,
    /// Send all queries to the mesh resolver (exit-node mode; Linux only).
    pub all: bool,
}

/// Source NAT for forwarded mesh traffic.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Nat {
    /// Mesh source ranges to masquerade.
    pub sources: Vec<IpPrefix>,
}
