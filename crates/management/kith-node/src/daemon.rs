use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kith_config::Config;
use kith_control::records::Relay;
use kith_control::{NetMap, Networks};
use kith_dataplane::{Dataplane, DataplaneHandle, Endpoint, PeerSpec};
use kith_identity::{Bootstrap, BootstrapAddr, DeviceIdentity, InviteToken, MAX_BOOTSTRAP};
use kith_ipc::{Event, NetworkSummary, PeerInfo, Request, Response, Status};
use kith_ledger::JoinProof;
use kith_policy::Addresses;
use kith_router::MeshRanges;
use kith_sys::{Dns, Interface, OsSystem, Platform, System};
use kith_transport_udp::UdpLink;
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::channel::{ChannelLayer, now, pick_targets};
use crate::connectivity::{Connectivity, Output};
use crate::dns::DnsService;
use crate::external::ExternalPeers;
use crate::ipc;
use crate::network::{Change, NetworkRuntime, NetworkSet};
use crate::os::{DynSystem, OsPlanner, PlanInputs};
use crate::server::{Listeners, ServerMode, stun_reply};
use crate::state::StateDir;
use crate::stealth::StealthOptions;
use crate::types::{Action, ChannelEvent, Command, ConnectivityEvent, NodeError};

const COMMAND_QUEUE: usize = 256;
const MEMORY_INTERFACE: &str = "kith0";

/// Which TUN device the node uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunMode {
    /// A real OS TUN device; needs privileges.
    #[default]
    System,
    /// An in-memory TUN driven through [`NodeHandle`], for tests.
    Memory,
}

/// How to start a [`Node`].
#[derive(Debug)]
pub struct NodeOptions {
    config: Config,
    tun: TunMode,
    system: DynSystem,
    memory_state: bool,
    listen_ip: IpAddr,
    dns_listen: Option<SocketAddr>,
    ipc: bool,
}

impl NodeOptions {
    /// Options for `config`, applying OS changes through `system`.
    pub fn new(config: Config, system: impl System + Send + Sync + 'static) -> Self {
        Self {
            config,
            tun: TunMode::System,
            system: DynSystem(Box::new(system)),
            memory_state: false,
            listen_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            dns_listen: None,
            ipc: true,
        }
    }

    /// Selects the TUN device.
    #[must_use]
    pub const fn tun(mut self, tun: TunMode) -> Self {
        self.tun = tun;
        self
    }

    /// Keeps identity and ledgers in memory instead of the state directory.
    #[must_use]
    pub const fn in_memory_state(mut self) -> Self {
        self.memory_state = true;
        self
    }

    /// Binds the WireGuard and control sockets on `ip` instead of every address.
    #[must_use]
    pub const fn listen_ip(mut self, ip: IpAddr) -> Self {
        self.listen_ip = ip;
        self
    }

    /// Serves DNS on `addr` instead of the networks' resolver address.
    #[must_use]
    pub const fn dns_listen(mut self, addr: SocketAddr) -> Self {
        self.dns_listen = Some(addr);
        self
    }

    /// Skips the IPC socket; requests then arrive only through [`NodeHandle::request`].
    #[must_use]
    pub const fn without_ipc(mut self) -> Self {
        self.ipc = false;
        self
    }

    /// Never sends WireGuard straight to peers, so every packet takes a relay.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub const fn without_direct_udp(mut self) -> Self {
        self.config.network.transports.stealth_only = true;
        self
    }
}

/// The kith daemon.
#[derive(Debug)]
pub struct Node;

impl Node {
    /// Starts every subsystem and the event loop on the current tokio runtime.
    pub async fn start(options: NodeOptions) -> Result<NodeHandle, NodeError> {
        let (commands, receiver) = mpsc::channel(COMMAND_QUEUE);
        let (daemon, started) = Daemon::build(options, commands.clone()).await?;
        Ok(NodeHandle {
            commands,
            task: tokio::spawn(daemon.run(receiver)),
            dns_addr: started.dns_addr,
            control_addr: started.control_addr,
            #[cfg(feature = "test-support")]
            tun: started.tun.map(tokio::sync::Mutex::new),
            #[cfg(feature = "test-support")]
            node: started.node,
            #[cfg(feature = "test-support")]
            relay_addr: started.relay_addr,
        })
    }
}

/// Controls a running [`Node`].
#[derive(Debug)]
pub struct NodeHandle {
    commands: mpsc::Sender<Command>,
    task: JoinHandle<Result<(), NodeError>>,
    dns_addr: Option<SocketAddr>,
    control_addr: SocketAddr,
    #[cfg(feature = "test-support")]
    tun: Option<tokio::sync::Mutex<kith_dataplane::MemoryTun>>,
    #[cfg(feature = "test-support")]
    node: NodeId,
    #[cfg(feature = "test-support")]
    relay_addr: Option<SocketAddr>,
}

impl NodeHandle {
    /// Handles an IPC request in process, as if a root client sent it.
    pub async fn request(&self, request: Request) -> Response {
        if let Some(response) = ipc::answer_locally(&request) {
            return response;
        }
        match ipc::translate(request) {
            Ok(action) => ipc::dispatch(&self.commands, action).await,
            Err(error) => ipc::error_response(&error),
        }
    }

    /// The address the DNS server is bound to, when DNS is enabled.
    pub const fn dns_addr(&self) -> Option<SocketAddr> {
        self.dns_addr
    }

    /// The address the control listener is bound to.
    pub const fn control_addr(&self) -> SocketAddr {
        self.control_addr
    }

    /// Writes an IP packet into the in-memory TUN, as if an application sent it.
    #[cfg(feature = "test-support")]
    pub async fn send_packet(&self, packet: Vec<u8>) -> Result<(), NodeError> {
        let tun = self.tun.as_ref().ok_or(NodeError::NoMemoryTun)?;
        Ok(tun.lock().await.inject(packet)?)
    }

    /// Reads the next IP packet the node delivered to the in-memory TUN.
    #[cfg(feature = "test-support")]
    pub async fn recv_packet(&self) -> Result<Vec<u8>, NodeError> {
        let tun = self.tun.as_ref().ok_or(NodeError::NoMemoryTun)?;
        tun.lock().await.receive().await.ok_or(NodeError::Stopped)
    }

    /// Advertises this node's relay in `network` as `relay/<name>`.
    #[cfg(feature = "test-support")]
    pub async fn advertise_relay(&self, network: NetworkId, name: &str) -> Result<(), NodeError> {
        let invalid =
            |error: &dyn core::fmt::Display| NodeError::InvalidArgument(error.to_string());
        let address = self
            .relay_addr
            .ok_or_else(|| NodeError::NotFound(String::from("relay")))?;
        let relay = Relay::new(name.to_owned(), address.to_string(), self.node)
            .map_err(|error| invalid(&error))?;
        let record = kith_control::records::Record::Relay(relay);
        let value = record.encode().map_err(|error| invalid(&error))?;
        let action = Action::SetRecord {
            network,
            key: record.key(),
            value,
        };
        let response = ipc::dispatch(&self.commands, action).await;
        if response == Response::RecordSet {
            Ok(())
        } else {
            Err(invalid(&format!("{response:?}")))
        }
    }

    /// Reverts OS changes and stops the node.
    pub async fn shutdown(self) -> Result<(), NodeError> {
        let (reply, done) = oneshot::channel();
        if self
            .commands
            .send(Command::Shutdown { reply })
            .await
            .is_ok()
        {
            done.await.map_err(|_| NodeError::Stopped)??;
        }
        self.task.await.map_err(|_| NodeError::Stopped)?
    }
}

/// Runs the real daemon until SIGINT or SIGTERM, then reverts OS changes.
pub async fn run(config: Config) -> Result<(), NodeError> {
    let node = Node::start(NodeOptions::new(config, OsSystem)).await?;
    shutdown_signal().await?;
    node.shutdown().await
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), NodeError> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), NodeError> {
    Ok(tokio::signal::ctrl_c().await?)
}

/// Addresses the started subsystems are bound to.
#[derive(Debug)]
pub(crate) struct Started {
    /// The DNS server, when enabled.
    pub(crate) dns_addr: Option<SocketAddr>,
    /// The control listener.
    pub(crate) control_addr: SocketAddr,
    /// The test end of the in-memory TUN.
    #[cfg(feature = "test-support")]
    pub(crate) tun: Option<kith_dataplane::MemoryTun>,
    /// This node's identity.
    #[cfg(feature = "test-support")]
    pub(crate) node: NodeId,
    /// The relay listener, in server mode.
    #[cfg(feature = "test-support")]
    pub(crate) relay_addr: Option<SocketAddr>,
}

/// The event loop's state; owned by one task, so nothing is locked.
#[derive(Debug)]
pub(crate) struct Daemon {
    identity: Arc<DeviceIdentity>,
    state: StateDir,
    networks: NetworkSet,
    channel: ChannelLayer,
    connectivity: Connectivity,
    dns: Option<DnsService>,
    dns_upstreams: Option<Vec<SocketAddr>>,
    addresses: BTreeMap<NetworkId, Vec<(IpAddr, u8)>>,
    commands: mpsc::Sender<Command>,
    os: OsPlanner,
    server: Option<ServerMode>,
    relay_listen: SocketAddr,
    external: ExternalPeers,
    external_names: BTreeSet<String>,
    dataplane: DataplaneHandle,
    dataplane_events: mpsc::Receiver<kith_dataplane::Event>,
    dataplane_task: JoinHandle<()>,
    events: broadcast::Sender<Event>,
    endpoints: BTreeMap<NodeId, Vec<SocketAddr>>,
    paths: BTreeMap<NodeId, Endpoint>,
    gossiped: Vec<SocketAddr>,
    exit: Option<(NetworkId, NodeId)>,
    ipc_task: Option<JoinHandle<()>>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.dataplane_task.abort();
        if let Some(task) = &self.ipc_task {
            task.abort();
        }
    }
}

struct Dataplanes {
    handle: DataplaneHandle,
    events: mpsc::Receiver<kith_dataplane::Event>,
    task: JoinHandle<()>,
    tun_name: Option<String>,
    #[cfg(feature = "test-support")]
    memory: Option<kith_dataplane::MemoryTun>,
}

fn start_dataplane(
    identity: &DeviceIdentity,
    link: UdpLink,
    tun: TunMode,
    config: &Config,
) -> Result<Dataplanes, NodeError> {
    let spawn = |dataplane: Dataplane| {
        tokio::spawn(async move {
            if let Err(error) = dataplane.run().await {
                tracing::warn!(%error, "dataplane stopped");
            }
        })
    };
    match tun {
        TunMode::System => {
            let tun = kith_tun::Tun::create(&kith_tun::TunConfig {
                name: config.network.tun_name.clone(),
                mtu: config.network.mtu,
                ipv4: None,
                ipv6: None,
                offload: true,
            })?;
            let tun_name = Some(tun.name().to_owned());
            let (dataplane, handle, events) = Dataplane::new(kith_dataplane::DataplaneConfig {
                signing_key: kith_crypto::SigningKey::from_seed(&identity.signing_key().to_seed()),
                tun,
                link,
            })?;
            Ok(Dataplanes {
                handle,
                events,
                task: spawn(dataplane),
                tun_name,
                #[cfg(feature = "test-support")]
                memory: None,
            })
        }
        #[cfg(feature = "test-support")]
        TunMode::Memory => {
            let (dataplane, handle, events, memory) =
                Dataplane::with_memory_tun(identity.signing_key(), link)?;
            Ok(Dataplanes {
                handle,
                events,
                task: spawn(dataplane),
                tun_name: None,
                memory: Some(memory),
            })
        }
        #[cfg(not(feature = "test-support"))]
        TunMode::Memory => Err(NodeError::NoMemoryTun),
    }
}

fn load_networks(state: &StateDir, local: NodeId) -> Result<NetworkSet, NodeError> {
    let mut networks = NetworkSet::default();
    for (id, ops) in state.load_networks()? {
        match NetworkRuntime::from_ops(ops, local) {
            Ok(runtime) => networks.insert(runtime)?,
            Err(error) => tracing::warn!(network = %id, %error, "skipping stored network"),
        }
    }
    Ok(networks)
}

fn local_addresses(map: &NetMap) -> Vec<(IpAddr, u8)> {
    let mut addresses = Vec::new();
    if let Some(local) = map.local_addresses() {
        addresses.push((IpAddr::V4(local.v4), map.ipv4_range().prefix_len()));
        addresses.push((IpAddr::V6(local.v6), map.ipv6_range().prefix_len()));
    }
    for resolver in map.resolver() {
        let host = if resolver.is_ipv4() { 32 } else { 128 };
        if !addresses.iter().any(|(address, _)| *address == resolver) {
            addresses.push((resolver, host));
        }
    }
    addresses
}

async fn start_dns(
    config: &Config,
    listen: Option<SocketAddr>,
) -> Result<(Option<Vec<SocketAddr>>, Option<DnsService>), NodeError> {
    if !config.dns.enabled {
        return Ok((None, None));
    }
    let upstreams: Vec<SocketAddr> = config.dns.fallback_upstreams.iter().map(|u| u.0).collect();
    let dns = match listen {
        Some(listen) => Some(DnsService::start(listen, upstreams.clone()).await?),
        None => None,
    };
    Ok((Some(upstreams), dns))
}

async fn restore_external(state: &StateDir, config: &Config) -> Result<ExternalPeers, NodeError> {
    let mut external = ExternalPeers::restore(state).await?;
    for import in &config.import_wireguard {
        external
            .import(state, import.name.clone(), import.load()?)
            .await?;
    }
    Ok(external)
}

fn start_ipc(
    config: &Config,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
) -> Result<JoinHandle<()>, NodeError> {
    let server = kith_ipc::IpcServer::bind(&config.daemon.socket_path)?;
    let uid = kith_sys::effective_uid().unwrap_or(0);
    Ok(tokio::spawn(async move {
        if let Err(error) = ipc::serve(server, commands, events, uid).await {
            tracing::warn!(%error, "ipc server stopped");
        }
    }))
}

impl Daemon {
    /// Loads state, creates the TUN and dataplane, and starts every subsystem.
    pub(crate) async fn build(
        options: NodeOptions,
        commands: mpsc::Sender<Command>,
    ) -> Result<(Self, Started), NodeError> {
        let config = &options.config;
        let state = if options.memory_state {
            StateDir::in_memory()
        } else {
            StateDir::open(&config.daemon.state_dir)?
        };
        let identity = Arc::new(state.load_or_create_identity()?);
        let local = identity.node_id();
        let networks = load_networks(&state, local)?;
        let link = UdpLink::bind(SocketAddr::new(
            options.listen_ip,
            config.network.listen_port,
        ))?;
        let bound = link.local_addr();
        let dataplanes = start_dataplane(&identity, link, options.tun, config)?;
        let channel = ChannelLayer::start(
            Arc::clone(&identity),
            networks.registry(),
            bound,
            commands.clone(),
        )
        .await?;
        let connectivity = Connectivity::start(
            Arc::clone(&identity),
            bound,
            dataplanes.tun_name.clone(),
            commands.clone(),
            StealthOptions::from_config(config),
        )?;
        let (dns_upstreams, dns) = start_dns(config, options.dns_listen).await?;
        let interface = Interface::new(dataplanes.tun_name.as_deref().unwrap_or(MEMORY_INTERFACE))?;
        let os = OsPlanner::new(
            options.system,
            Platform::current()?,
            interface,
            &config.daemon.state_dir,
        );
        let relay_listen = SocketAddr::new(options.listen_ip, bound.port().saturating_add(1));
        let server = if config.server.enabled {
            Some(
                ServerMode::start(
                    &identity,
                    Listeners::from_config(&config.server, relay_listen),
                    config.server.exit,
                    connectivity.quic_links(),
                    commands.clone(),
                )
                .await?,
            )
        } else {
            None
        };
        let external = restore_external(&state, config).await?;
        let (events, _) = broadcast::channel(ipc::EVENT_BUFFER);
        let ipc_task = if options.ipc {
            Some(start_ipc(config, commands.clone(), events.clone())?)
        } else {
            None
        };
        let started = Started {
            dns_addr: dns.as_ref().map(DnsService::local_addr),
            control_addr: channel.local_addr(),
            #[cfg(feature = "test-support")]
            tun: dataplanes.memory,
            #[cfg(feature = "test-support")]
            node: local,
            #[cfg(feature = "test-support")]
            relay_addr: server.as_ref().and_then(ServerMode::relay_addr),
        };
        let mut daemon = Self {
            identity,
            state,
            networks,
            channel,
            connectivity,
            dns,
            dns_upstreams,
            addresses: BTreeMap::new(),
            commands,
            os,
            server,
            relay_listen,
            external,
            external_names: BTreeSet::new(),
            dataplane: dataplanes.handle,
            dataplane_events: dataplanes.events,
            dataplane_task: dataplanes.task,
            events,
            endpoints: BTreeMap::new(),
            paths: BTreeMap::new(),
            gossiped: Vec::new(),
            exit: None,
            ipc_task,
        };
        daemon.refresh(None).await?;
        Ok((daemon, started))
    }

    /// Runs the loop until shutdown or until every command sender is gone.
    pub(crate) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
    ) -> Result<(), NodeError> {
        loop {
            let deadline = self.connectivity.poll_timeout();
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown { reply }) => {
                        let result = self.shutdown();
                        let failed = result.is_err();
                        let _sent = reply.send(result);
                        return if failed { Err(NodeError::Stopped) } else { Ok(()) };
                    }
                    Some(command) => self.handle(command).await,
                    None => return self.shutdown(),
                },
                event = self.dataplane_events.recv() => {
                    let Some(event) = event else {
                        self.shutdown()?;
                        return Err(NodeError::Stopped);
                    };
                    let handled = self.on_dataplane(event).await;
                    self.report(handled);
                    let flushed = self.flush_outputs().await;
                    self.report(flushed);
                }
                () = sleep_until(deadline) => {
                    self.connectivity.handle_timeout(Instant::now());
                    let flushed = self.flush_outputs().await;
                    self.report(flushed);
                }
            }
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Local { action, reply } => {
                let _sent = reply.send(self.execute(action).await);
            }
            Command::Channel(event) => {
                let handled = self.on_channel(event).await;
                self.report(handled);
            }
            Command::Connectivity(event) => {
                let handled = self.on_connectivity(event);
                self.report(handled);
            }
            Command::Shutdown { reply } => {
                let _sent = reply.send(self.shutdown());
            }
        }
        let flushed = self.flush_outputs().await;
        self.report(flushed);
    }

    fn report(&self, result: Result<(), NodeError>) {
        if let Err(error) = result {
            tracing::warn!(%error, "node event failed");
            let _receivers = self.events.send(Event::Warning {
                message: error.to_string(),
            });
        }
    }

    fn runtime_mut(&mut self, network: NetworkId) -> Result<&mut NetworkRuntime, NodeError> {
        self.networks
            .get_mut(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))
    }

    fn member_map(&self, network: NetworkId) -> Result<&NetMap, NodeError> {
        self.networks
            .maps()
            .get(&network)
            .filter(|map| map.is_member())
            .ok_or_else(|| NodeError::NotFound(network.to_string()))
    }

    /// Performs a local action and builds its IPC response.
    pub(crate) async fn execute(&mut self, action: Action) -> Result<Response, NodeError> {
        let identity = Arc::clone(&self.identity);
        match action {
            Action::Status => self.status().await,
            Action::CreateNetwork { name, device_name } => {
                self.create_network(&name, &device_name).await
            }
            Action::CreateInvite {
                network,
                expires_in,
                max_uses,
            } => self.create_invite(network, expires_in, max_uses).await,
            Action::Join { token, device_name } => self.join(&token, &device_name).await,
            Action::Leave { network } => self.leave(network).await,
            Action::Peers { network } => self.peers(network).await,
            Action::SetRecord {
                network,
                key,
                value,
            } => {
                self.runtime_mut(network)?
                    .set_record(&identity, key, value, now())?;
                self.commit(network).await?;
                Ok(Response::RecordSet)
            }
            Action::DeleteRecord { network, key } => {
                self.runtime_mut(network)?
                    .set_record(&identity, key, Vec::new(), now())?;
                self.commit(network).await?;
                Ok(Response::RecordDeleted)
            }
            Action::SetPolicy { network, policy } => {
                self.runtime_mut(network)?
                    .set_policy(&identity, &policy, now())?;
                self.commit(network).await?;
                Ok(Response::PolicySet)
            }
            Action::UseExit { network, node } => self.use_exit(network, node).await,
            Action::ServerMode { exit, relay } => self.server_mode(exit, relay).await,
            Action::ImportWireGuard { name, config } => {
                let network = self.external.import(&self.state, name, *config).await?;
                self.refresh(None).await?;
                Ok(Response::Imported { network })
            }
            Action::ExportWireGuard {
                network,
                device_name,
            } => self.export(network, &device_name).await,
        }
    }

    async fn create_network(
        &mut self,
        name: &str,
        device_name: &str,
    ) -> Result<Response, NodeError> {
        let runtime = NetworkRuntime::create(&self.identity, name, device_name, now())?;
        let network = runtime.id();
        if self.networks.get(&network).is_some() {
            return Err(NodeError::InvalidArgument(format!(
                "network {network} exists"
            )));
        }
        self.networks.insert(runtime)?;
        self.persist(network)?;
        self.refresh(Some(network)).await?;
        Ok(Response::NetworkCreated { network })
    }

    async fn create_invite(
        &mut self,
        network: NetworkId,
        expires_in: core::time::Duration,
        max_uses: u16,
    ) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let bootstrap = self.bootstrap();
        let identity = Arc::clone(&self.identity);
        let token =
            self.runtime_mut(network)?
                .invite(&identity, expires_in, max_uses, bootstrap, now())?;
        self.commit(network).await?;
        Ok(Response::Invite {
            invite_link: token.to_link()?.to_string(),
        })
    }

    async fn join(
        &mut self,
        token: &InviteToken,
        device_name: &str,
    ) -> Result<Response, NodeError> {
        let network = token.network();
        if self.networks.get(&network).is_some() {
            return Err(NodeError::InvalidArgument(format!(
                "already joined {network}"
            )));
        }
        let proof = JoinProof::create(
            network,
            &token.invite_key(),
            self.identity.signing_key(),
            device_name,
        )?;
        let ledger = self.channel.join(token, proof).await?;
        if ledger.network_id() != network {
            return Err(NodeError::InvalidArgument(String::from(
                "the sponsor returned another network",
            )));
        }
        self.networks.insert(NetworkRuntime::from_ledger(
            ledger,
            self.identity.node_id(),
        )?)?;
        for entry in token.bootstrap() {
            if let BootstrapAddr::Socket(addr) = entry.addr() {
                let known = self.endpoints.entry(entry.node()).or_default();
                if !known.contains(addr) {
                    known.push(*addr);
                }
            }
        }
        self.persist(network)?;
        self.refresh(Some(network)).await?;
        Ok(Response::Joined { network })
    }

    async fn leave(&mut self, network: NetworkId) -> Result<Response, NodeError> {
        if self.networks.remove(&network)?.is_none() {
            return Err(NodeError::NotFound(network.to_string()));
        }
        self.state.remove_network(network)?;
        self.dataplane.remove_network(network).await?;
        self.addresses.remove(&network);
        if self.exit.is_some_and(|(exit, _)| exit == network) {
            self.exit = None;
            self.dataplane.set_exit(None).await?;
        }
        self.refresh(None).await?;
        Ok(Response::Left)
    }

    async fn use_exit(
        &mut self,
        network: NetworkId,
        node: Option<NodeId>,
    ) -> Result<Response, NodeError> {
        let map = self.member_map(network)?;
        if let Some(node) = node
            && !map.exit_nodes().contains(&node)
        {
            return Err(NodeError::InvalidArgument(format!(
                "{node} is not an exit node"
            )));
        }
        self.exit = node.map(|node| (network, node));
        self.dataplane.set_exit(self.exit).await?;
        self.refresh(None).await?;
        Ok(Response::ExitSet)
    }

    async fn server_mode(&mut self, exit: bool, relay: bool) -> Result<Response, NodeError> {
        self.server = None;
        if exit || relay {
            self.server = Some(
                ServerMode::start(
                    &self.identity,
                    Listeners::relay_only(relay.then_some(self.relay_listen)),
                    exit,
                    self.connectivity.quic_links(),
                    self.commands.clone(),
                )
                .await?,
            );
        }
        self.refresh(None).await?;
        Ok(Response::ServerModeSet)
    }

    async fn export(
        &mut self,
        network: NetworkId,
        device_name: &str,
    ) -> Result<Response, NodeError> {
        let endpoint = self
            .connectivity
            .local_endpoints()
            .first()
            .copied()
            .unwrap_or_else(|| self.channel.local_addr())
            .to_string();
        let runtime = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?;
        let conf_text = self
            .external
            .export(&self.state, runtime, device_name, &endpoint)?;
        self.refresh(None).await?;
        Ok(Response::WireGuardConfig { conf_text })
    }

    fn bootstrap(&self) -> Vec<Bootstrap> {
        let control = self.channel.local_addr();
        let addrs: Vec<SocketAddr> = if control.ip().is_unspecified() {
            self.connectivity
                .local_endpoints()
                .into_iter()
                .map(|addr| SocketAddr::new(addr.ip(), control.port()))
                .collect()
        } else {
            vec![control]
        };
        let node = self.identity.node_id();
        let mut bootstrap: Vec<Bootstrap> = Vec::new();
        for addr in addrs {
            let entry = Bootstrap::new(node, BootstrapAddr::Socket(addr));
            if !bootstrap.contains(&entry) && entry.validate().is_ok() {
                bootstrap.push(entry);
            }
        }
        bootstrap.truncate(MAX_BOOTSTRAP);
        bootstrap
    }

    async fn status(&self) -> Result<Response, NodeError> {
        let stats = self.dataplane.stats().await?;
        let networks = self
            .networks
            .maps()
            .iter()
            .filter(|(_, map)| map.is_member())
            .map(|(id, map)| NetworkSummary {
                id: *id,
                name: map.name().to_owned(),
                peers_online: count(
                    stats
                        .peers
                        .iter()
                        .filter(|peer| peer.network == *id && peer.last_handshake.is_some()),
                ),
                peers_total: count(map.peers().iter()),
            })
            .collect();
        Ok(Response::Status(Status {
            node: Some(self.identity.node_id()),
            networks,
            exit: self.server.as_ref().is_some_and(ServerMode::is_exit),
            relay: self
                .server
                .as_ref()
                .is_some_and(|server| server.relay_addr().is_some()),
        }))
    }

    async fn peers(&self, network: NetworkId) -> Result<Response, NodeError> {
        let map = self.member_map(network)?;
        let stats = self.dataplane.stats().await?;
        let at = wall_instant();
        let peers = map
            .peers()
            .iter()
            .map(|(node, peer)| {
                let stat = stats
                    .peers
                    .iter()
                    .find(|stat| stat.network == network && stat.node == *node);
                PeerInfo {
                    node: *node,
                    name: peer.name.clone(),
                    address: peer.addresses.v4.to_string(),
                    path: self.connectivity.path_kind(node),
                    last_handshake_secs: stat
                        .and_then(|stat| stat.last_handshake)
                        .map(|when| at.saturating_duration_since(when).as_secs()),
                    endpoint: stat
                        .and_then(|stat| stat.endpoint)
                        .map(|endpoint| match endpoint {
                            Endpoint::Direct(addr) => addr.to_string(),
                            Endpoint::Relay => String::from("relay"),
                        }),
                    rx_bytes: stat.map_or(0, |stat| stat.rx_bytes),
                    tx_bytes: stat.map_or(0, |stat| stat.tx_bytes),
                }
            })
            .collect();
        Ok(Response::Peers(peers))
    }

    fn persist(&self, network: NetworkId) -> Result<(), NodeError> {
        let runtime = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?;
        runtime.with_ledger(|ledger| self.state.save_network(ledger))?
    }

    async fn commit(&mut self, network: NetworkId) -> Result<(), NodeError> {
        if let Some(change) = self.networks.rebuild(&network, self.identity.node_id())? {
            return self.apply_change(change).await;
        }
        self.persist(network)?;
        self.channel.notify_changed(network);
        Ok(())
    }

    fn add_known_candidates(&mut self) {
        let at = Instant::now();
        for (node, addrs) in &self.endpoints {
            self.connectivity.add_candidates(*node, addrs, at);
        }
    }

    /// Handles ledger changes and endpoint gossip reported by the control channel.
    pub(crate) async fn on_channel(&mut self, event: ChannelEvent) -> Result<(), NodeError> {
        match event {
            ChannelEvent::LedgerChanged { network } => {
                if self.networks.get(&network).is_none() {
                    return Ok(());
                }
                match self.networks.rebuild(&network, self.identity.node_id())? {
                    Some(change) => self.apply_change(change).await,
                    None => self.persist(network),
                }
            }
            ChannelEvent::Endpoints {
                network,
                from,
                entries,
            } => {
                let Some(map) = self.networks.maps().get(&network) else {
                    return Ok(());
                };
                if !map.peers().contains_key(&from) {
                    return Ok(());
                }
                let local = self.identity.node_id();
                let accepted: Vec<(NodeId, Vec<SocketAddr>)> = entries
                    .into_iter()
                    .filter(|(node, _)| *node != local && map.peers().contains_key(node))
                    .collect();
                let at = Instant::now();
                let mut changed = false;
                for (node, addrs) in accepted {
                    self.connectivity.add_candidates(node, &addrs, at);
                    let known = self.endpoints.entry(node).or_default();
                    for addr in addrs.iter().rev() {
                        if !known.contains(addr) {
                            known.insert(0, *addr);
                            changed = true;
                        }
                    }
                    known.truncate(MAX_BOOTSTRAP);
                }
                if changed {
                    self.update_targets()?;
                }
                Ok(())
            }
        }
    }

    /// Handles relayed datagrams, host network changes and port mappings.
    pub(crate) fn on_connectivity(&mut self, event: ConnectivityEvent) -> Result<(), NodeError> {
        let at = Instant::now();
        match event {
            ConnectivityEvent::Relayed { from, datagram } => {
                self.connectivity.handle_relayed(from, datagram, at)?;
            }
            ConnectivityEvent::NetworkChanged(change) => {
                self.connectivity.handle_network_change(&change, at);
            }
            ConnectivityEvent::PortMapped(mapped) => self.connectivity.set_port_mapping(mapped),
        }
        self.gossip()
    }

    /// Routes dataplane events: STUN to the server or connectivity, relay sends, handshakes.
    pub(crate) async fn on_dataplane(
        &mut self,
        event: kith_dataplane::Event,
    ) -> Result<(), NodeError> {
        let at = Instant::now();
        match event {
            kith_dataplane::Event::Control { source, datagram } => {
                if self.server.is_some()
                    && let Some(reply) = stun_reply(source, &datagram)
                {
                    return Ok(self.dataplane.send_raw(source, reply).await?);
                }
                self.connectivity.handle_datagram(source, &datagram, at)?;
                self.gossip()
            }
            kith_dataplane::Event::Relay { peer, datagram } => {
                self.connectivity.send_relayed(peer, datagram)
            }
            kith_dataplane::Event::HandshakeCompleted { peer } => {
                self.connectivity.note_handshake(peer, at);
                if let Some(path) = self.connectivity.path_kind(&peer) {
                    for (network, _) in self.maps_with(peer) {
                        let _receivers = self.events.send(Event::PeerConnected {
                            network,
                            node: peer,
                            path,
                        });
                    }
                }
                Ok(())
            }
            kith_dataplane::Event::SessionExpired { peer } => {
                for (network, _) in self.maps_with(peer) {
                    let _receivers = self.events.send(Event::PeerDisconnected {
                        network,
                        node: peer,
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn maps_with(&self, peer: NodeId) -> Vec<(NetworkId, &NetMap)> {
        self.networks
            .maps()
            .iter()
            .filter(|(_, map)| map.peers().contains_key(&peer))
            .map(|(id, map)| (*id, map))
            .collect()
    }

    /// Persists the ledger and pushes a map change to the dataplane, DNS, OS and channel.
    pub(crate) async fn apply_change(&mut self, change: Change) -> Result<(), NodeError> {
        let network = change.network;
        self.persist(network)?;
        self.refresh(Some(network)).await?;
        if change.diff.local_device_changed || change.diff.dns_changed {
            let _receivers = self.events.send(Event::NetworkChanged { network });
        }
        self.channel.notify_changed(network);
        Ok(())
    }

    async fn refresh(&mut self, changed: Option<NetworkId>) -> Result<(), NodeError> {
        self.push_peers().await?;
        let maps = self.networks.maps();
        for (id, map) in maps.iter().filter(|(_, map)| map.is_member()) {
            if changed.is_some_and(|network| network != *id) {
                continue;
            }
            self.dataplane
                .set_routes(*id, mesh_ranges(map), mesh_routes(map)?)
                .await?;
            self.dataplane.set_filter(*id, map.filter().clone()).await?;
        }
        for (id, map) in member_maps_with_ids(maps) {
            let wanted = local_addresses(map);
            if self.addresses.get(&id) != Some(&wanted) {
                self.dataplane.set_addresses(id, wanted.clone()).await?;
                self.addresses.insert(id, wanted);
            }
        }
        let peers: Vec<NodeId> = member_maps(maps)
            .flat_map(|map| map.peers().keys().copied())
            .collect();
        let relays = relays(maps);
        let at = Instant::now();
        self.connectivity.set_peers(&peers, at);
        self.connectivity.set_relays(&relays)?;
        self.add_known_candidates();
        self.update_targets()?;
        if let Some(server) = &self.server {
            let members: BTreeSet<NodeId> = peers
                .iter()
                .copied()
                .chain(core::iter::once(self.identity.node_id()))
                .collect();
            server.set_members(members).await?;
        }
        self.ensure_dns().await;
        if let Some(dns) = &self.dns {
            dns.update(self.networks.maps())?;
        }
        self.push_external().await?;
        self.apply_os()?;
        self.gossiped.clear();
        self.gossip()
    }

    async fn ensure_dns(&mut self) {
        let Some(upstreams) = self.dns_upstreams.as_ref().filter(|_| self.dns.is_none()) else {
            return;
        };
        let Some((_, map)) = member_maps_with_ids(self.networks.maps()).next() else {
            return;
        };
        let listen = SocketAddr::new(map.resolver()[0], 53);
        match DnsService::start(listen, upstreams.clone()).await {
            Ok(dns) => {
                tracing::debug!(%listen, "DNS listening");
                self.dns = Some(dns);
            }
            Err(error) => tracing::warn!(%listen, %error, "DNS server did not start"),
        }
    }

    async fn push_peers(&self) -> Result<(), NodeError> {
        let mut specs = Vec::new();
        for (id, map) in self
            .networks
            .maps()
            .iter()
            .filter(|(_, map)| map.is_member())
        {
            for (node, peer) in map.peers() {
                let mut allowed_ips = host_prefixes(peer.addresses)?.to_vec();
                allowed_ips.extend(
                    map.routes()
                        .iter()
                        .filter(|(_, owner)| *owner == node)
                        .map(|(prefix, _)| *prefix),
                );
                specs.push(PeerSpec {
                    network: *id,
                    node: *node,
                    allowed_ips,
                    endpoint: self.paths.get(node).copied(),
                    persistent_keepalive: None,
                });
            }
        }
        Ok(self.dataplane.set_peers(specs).await?)
    }

    async fn push_external(&mut self) -> Result<(), NodeError> {
        let interfaces = self.external.interfaces()?;
        let names: BTreeSet<String> = interfaces.iter().map(|i| i.name.clone()).collect();
        for interface in interfaces {
            self.dataplane.set_external(interface).await?;
        }
        for gone in self.external_names.difference(&names) {
            self.dataplane.remove_external(gone.clone()).await?;
        }
        self.external_names = names;
        Ok(())
    }

    fn apply_os(&mut self) -> Result<(), NodeError> {
        let maps = self.networks.maps();
        let exit_endpoints = self
            .exit
            .and_then(|(_, node)| self.paths.get(&node))
            .and_then(|endpoint| match endpoint {
                Endpoint::Direct(addr) => Some(addr.ip()),
                Endpoint::Relay => None,
            })
            .into_iter()
            .collect();
        let dns = self.dns.as_ref().and_then(|dns| {
            let domains = dns.domains(maps);
            (!domains.is_empty()).then(|| Dns {
                resolver: dns.local_addr().ip(),
                domains,
                all: false,
            })
        });
        let inputs = PlanInputs {
            exit: self.exit,
            exit_endpoints,
            dns,
            serve_exit: self.server.as_ref().is_some_and(ServerMode::is_exit),
            extra_routes: self.external.routes(),
        };
        let plan = self.os.plan(maps, &inputs)?;
        self.os.apply(plan)
    }

    fn update_targets(&mut self) -> Result<(), NodeError> {
        let maps = self.networks.maps();
        let members: Vec<&NetMap> = member_maps(maps).collect();
        let targets = pick_targets(&members, &relays(maps), &self.endpoints);
        self.channel.set_targets(targets)
    }

    fn gossip(&mut self) -> Result<(), NodeError> {
        let endpoints = self.connectivity.local_endpoints();
        if endpoints == self.gossiped {
            return Ok(());
        }
        let local = self.identity.node_id();
        for runtime in self.networks.iter() {
            if runtime.netmap().is_member() {
                self.channel
                    .gossip(runtime.id(), vec![(local, endpoints.clone())])?;
            }
        }
        self.gossiped = endpoints;
        Ok(())
    }

    /// Executes queued connectivity outputs against the dataplane.
    pub(crate) async fn flush_outputs(&mut self) -> Result<(), NodeError> {
        let mut paths_changed = false;
        for output in self.connectivity.drain_outputs() {
            match output {
                Output::SendRaw {
                    destination,
                    datagram,
                } => self.dataplane.send_raw(destination, datagram).await?,
                Output::AcceptRelayed { peer, datagram } => {
                    self.dataplane.accept_relayed(peer, datagram).await?;
                }
                Output::PathChanged { peer, endpoint } => {
                    tracing::debug!(%peer, ?endpoint, "peer path changed");
                    paths_changed |= self.paths.insert(peer, endpoint) != Some(endpoint);
                }
            }
        }
        if paths_changed {
            self.push_peers().await?;
            if self.exit.is_some() {
                self.apply_os()?;
            }
        }
        Ok(())
    }

    /// Reverts the OS plan.
    pub(crate) fn shutdown(&mut self) -> Result<(), NodeError> {
        self.os.revert()
    }
}

fn member_maps(maps: &Networks) -> impl Iterator<Item = &NetMap> {
    maps.iter()
        .map(|(_, map)| map)
        .filter(|map| map.is_member())
}

fn member_maps_with_ids(maps: &Networks) -> impl Iterator<Item = (NetworkId, &NetMap)> {
    maps.iter()
        .filter(|(_, map)| map.is_member())
        .map(|(id, map)| (*id, map))
}

fn relays(maps: &Networks) -> Vec<Relay> {
    let mut relays: Vec<Relay> = Vec::new();
    for relay in member_maps(maps).flat_map(NetMap::relays) {
        if !relays.iter().any(|known| known.node() == relay.node()) {
            relays.push(relay.clone());
        }
    }
    relays
}

const fn mesh_ranges(map: &NetMap) -> MeshRanges {
    MeshRanges {
        v4: map.ipv4_range(),
        v6: map.ipv6_range(),
    }
}

fn mesh_routes(map: &NetMap) -> Result<Vec<(IpPrefix, NodeId)>, NodeError> {
    let mut routes = Vec::new();
    for (node, peer) in map.peers() {
        for prefix in host_prefixes(peer.addresses)? {
            routes.push((prefix, *node));
        }
    }
    routes.extend(
        map.routes()
            .iter()
            .filter(|(_, owner)| map.peers().contains_key(*owner))
            .map(|(prefix, owner)| (*prefix, *owner)),
    );
    Ok(routes)
}

fn host_prefixes(addresses: Addresses) -> Result<[IpPrefix; 2], NodeError> {
    let invalid = |error: kith_types::PrefixError| NodeError::InvalidArgument(error.to_string());
    Ok([
        IpPrefix::V4(Ipv4Prefix::new(addresses.v4, 32).map_err(invalid)?),
        IpPrefix::V6(Ipv6Prefix::new(addresses.v6, 128).map_err(invalid)?),
    ])
}

#[expect(
    clippy::disallowed_methods,
    reason = "handshake ages are measured against the dataplane's std clock"
)]
fn wall_instant() -> std::time::Instant {
    std::time::Instant::now()
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => core::future::pending().await,
    }
}
