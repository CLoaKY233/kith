#![expect(
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::net::SocketAddr;
use std::sync::Arc;

use kith_control::Networks;
use kith_dns::{DnsServer, DnsState};
use kith_naming::Zone;
use kith_sys::Domain;
use tokio::task::JoinHandle;

use crate::types::NodeError;

/// The DNS server answering for every joined network on the resolver address.
#[derive(Debug)]
pub(crate) struct DnsService {
    state: Arc<DnsState>,
    local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl DnsService {
    /// Binds the server on `listen`, forwarding other names to `upstreams`.
    pub(crate) async fn start(
        listen: SocketAddr,
        upstreams: Vec<SocketAddr>,
    ) -> Result<Self, NodeError> {
        let state = Arc::new(DnsState::new(Zone::empty()));
        state.set_default_upstreams(upstreams);
        let server = DnsServer::bind(listen, Arc::clone(&state)).await?;
        let local_addr = server.local_addr()?;
        let task = tokio::spawn(server.run());
        Ok(Self {
            state,
            local_addr,
            task,
        })
    }

    /// The bound address.
    pub(crate) const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Rebuilds the zone and split-DNS routes from every network map.
    pub(crate) fn update(&self, maps: &Networks) -> Result<(), NodeError> {
        self.state.set_zone(build_zone(maps)?);
        Ok(())
    }

    /// The domains the OS must send to this server.
    pub(crate) fn domains(&self, maps: &Networks) -> Vec<Domain> {
        Vec::new()
    }
}

impl Drop for DnsService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Builds one zone from device names and `dns/` records of every network.
pub(crate) fn build_zone(maps: &Networks) -> Result<Zone, NodeError> {
    Err(NodeError::Pending("dns: build zone"))
}
