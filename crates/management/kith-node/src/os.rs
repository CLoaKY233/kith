use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;

use kith_control::Networks;
use kith_sys::Dns;
use kith_sys::{
    Cmd, Error, ExitNode, Interface, Nat, Output, Plan, Platform, RouteConflict, Sys, System,
};
use kith_types::{IpPrefix, NetworkId, NodeId};

use crate::types::NodeError;

/// A boxed [`System`], so the node is not generic over it.
pub(crate) struct DynSystem(pub(crate) Box<dyn System + Send + Sync>);

impl core::fmt::Debug for DynSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynSystem").finish_non_exhaustive()
    }
}

impl System for DynSystem {
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        self.0.run(cmd)
    }

    fn read(&mut self, path: &Path) -> Result<Option<String>, Error> {
        self.0.read(path)
    }

    fn write(&mut self, path: &Path, contents: &str) -> Result<(), Error> {
        self.0.write(path, contents)
    }

    fn remove(&mut self, path: &Path) -> Result<(), Error> {
        self.0.remove(path)
    }

    fn list(&mut self, dir: &Path) -> Result<Vec<std::path::PathBuf>, Error> {
        self.0.list(dir)
    }

    fn has_program(&mut self, name: &str) -> bool {
        self.0.has_program(name)
    }
}

/// What the OS plan depends on besides the network maps.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanInputs {
    /// The exit peer in use, if any.
    pub(crate) exit: Option<(NetworkId, NodeId)>,
    /// Underlay addresses of the exit peer, kept off the tunnel.
    pub(crate) exit_endpoints: Vec<core::net::IpAddr>,
    /// Split-DNS settings, when DNS is enabled.
    pub(crate) dns: Option<Dns>,
    /// Server mode with exit: forward and NAT mesh traffic.
    pub(crate) serve_exit: bool,
    /// Extra routes, such as imported WireGuard exits.
    pub(crate) extra_routes: Vec<IpPrefix>,
}

/// Applies the OS plan derived from the network maps and reverts it on shutdown.
#[derive(Debug)]
pub(crate) struct OsPlanner {
    sys: Sys<DynSystem>,
    interface: Interface,
    applied: Option<Plan>,
    conflicts: Vec<RouteConflict>,
}

impl OsPlanner {
    /// Drives `system` on `platform` for TUN `interface`, keeping restore data under `state_dir`.
    pub(crate) fn new(
        system: DynSystem,
        platform: Platform,
        interface: Interface,
        state_dir: &Path,
    ) -> Self {
        Self {
            sys: Sys::new(system, platform, state_dir),
            interface,
            applied: None,
            conflicts: Vec::new(),
        }
    }

    /// Derives the plan: mesh and subnet routes, DNS, exit routing, forwarding and NAT.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature kept for the daemon builder"
    )]
    pub(crate) fn plan(&self, maps: &Networks, inputs: &PlanInputs) -> Result<Plan, NodeError> {
        let mut routes: BTreeSet<IpPrefix> = inputs.extra_routes.iter().copied().collect();
        let mut mesh = BTreeSet::new();
        let mut routes_local_subnets = false;
        for (_, map) in maps.iter().filter(|(_, map)| map.is_member()) {
            mesh.insert(IpPrefix::V4(map.ipv4_range()));
            mesh.insert(IpPrefix::V6(map.ipv6_range()));
            for (prefix, owner) in map.routes() {
                if map.peers().contains_key(owner) {
                    routes.insert(*prefix);
                } else {
                    routes_local_subnets = true;
                }
            }
        }
        routes.extend(mesh.iter().copied());
        let exit_node = inputs.exit.map(|_| ExitNode {
            peer_endpoints: inputs.exit_endpoints.clone(),
        });
        let dns = inputs.dns.clone().map(|dns| Dns {
            all: dns.all || exit_node.is_some(),
            ..dns
        });
        let nat = inputs.serve_exit.then(|| Nat {
            sources: mesh.into_iter().collect(),
        });
        Ok(Plan {
            interface: self.interface.clone(),
            routes: routes.into_iter().collect(),
            exit_node,
            dns,
            forwarding: inputs.serve_exit || routes_local_subnets,
            nat,
        })
    }

    /// Applies `plan` if it differs from the applied one.
    pub(crate) fn apply(&mut self, plan: Plan) -> Result<(), NodeError> {
        if self.applied.as_ref() == Some(&plan) {
            return Ok(());
        }
        self.check_conflicts(&plan.routes);
        self.sys.change(self.applied.as_ref(), &plan)?;
        self.applied = Some(plan);
        Ok(())
    }

    /// Routes other interfaces hold inside kith's ranges, as of the last plan.
    pub(crate) fn conflicts(&self) -> &[RouteConflict] {
        &self.conflicts
    }

    fn check_conflicts(&mut self, ranges: &[IpPrefix]) {
        let found = match self.sys.route_conflicts(ranges, &self.interface) {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "could not read the routing table");
                return;
            }
        };
        if found == self.conflicts {
            return;
        }
        for conflict in found.iter().filter(|c| !self.conflicts.contains(c)) {
            tracing::warn!(
                %conflict,
                "another interface routes addresses in kith's range, usually another VPN such as Tailscale; \
                 kith peers at those addresses will be unreachable"
            );
        }
        self.conflicts = found;
    }

    /// The host's own DNS servers on port 53, for names outside the mesh.
    pub(crate) fn system_resolvers(&mut self) -> Vec<SocketAddr> {
        match self.sys.system_resolvers() {
            Ok(ips) => ips.into_iter().map(|ip| SocketAddr::new(ip, 53)).collect(),
            Err(error) => {
                tracing::warn!(%error, "could not read the system DNS servers");
                Vec::new()
            }
        }
    }

    /// Reverts the applied plan, if any.
    pub(crate) fn revert(&mut self) -> Result<(), NodeError> {
        if let Some(plan) = self.applied.take() {
            self.sys.revert(&plan)?;
        }
        Ok(())
    }
}
