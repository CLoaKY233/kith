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
    /// Addresses of standard WireGuard clients, such as phones, served by this device.
    pub(crate) gateway: Option<IpPrefix>,
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
        routes.extend(inputs.gateway);
        let exit_node = inputs.exit.map(|_| ExitNode {
            peer_endpoints: inputs.exit_endpoints.clone(),
        });
        let dns = inputs.dns.clone().map(|dns| Dns {
            all: dns.all || exit_node.is_some(),
            ..dns
        });
        let nat = (inputs.serve_exit || inputs.gateway.is_some()).then(|| Nat {
            sources: if inputs.serve_exit {
                mesh.into_iter().chain(inputs.gateway).collect()
            } else {
                Vec::new()
            },
            into_tunnel: inputs.gateway.into_iter().collect(),
        });
        let exit_network = inputs.exit.map(|(network, _)| network);
        let mut members: Vec<_> = maps.iter().filter(|(_, map)| map.is_member()).collect();
        members.sort_by_key(|(id, _)| Some(**id) != exit_network);
        let sources = members
            .iter()
            .filter_map(|(_, map)| map.local_device())
            .flat_map(|device| [device.addresses.v4.into(), device.addresses.v6.into()])
            .collect();
        Ok(Plan {
            interface: self.interface.clone(),
            routes: routes.into_iter().collect(),
            exit_node,
            dns,
            forwarding: inputs.serve_exit || routes_local_subnets || inputs.gateway.is_some(),
            nat,
            sources,
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

#[cfg(test)]
mod tests {
    use kith_control::Networks;
    use kith_sys::{Interface, NoopSystem, Platform};
    use kith_types::IpPrefix;

    use super::{DynSystem, OsPlanner, PlanInputs};

    #[test]
    fn a_gateway_routes_forwards_and_masquerades_its_clients() {
        let planner = OsPlanner::new(
            DynSystem(Box::new(NoopSystem)),
            Platform::Linux,
            Interface::new("kith0").unwrap(),
            &std::env::temp_dir(),
        );
        let range: IpPrefix = "10.225.0.0/24".parse().unwrap();
        let none = planner
            .plan(&Networks::new(), &PlanInputs::default())
            .unwrap();
        assert!(none.nat.is_none() && !none.forwarding && none.routes.is_empty());

        let inputs = PlanInputs {
            gateway: Some(range),
            ..PlanInputs::default()
        };
        let gateway = planner.plan(&Networks::new(), &inputs).unwrap();
        assert!(gateway.routes.contains(&range) && gateway.forwarding);
        let nat = gateway.nat.unwrap();
        assert_eq!(nat.into_tunnel, [range]);
        assert!(
            nat.sources.is_empty(),
            "no exit, so nothing goes to the internet"
        );

        let exit = PlanInputs {
            serve_exit: true,
            ..inputs
        };
        let nat = planner.plan(&Networks::new(), &exit).unwrap().nat.unwrap();
        assert!(nat.sources.contains(&range), "clients use the exit too");
    }
}
