use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use kith_channel::Presence;
use kith_config::Config;
use kith_control::records::{Record, Relay};
use kith_control::{NetMap, Networks, Warning as MapWarning};
use kith_crypto::AgreementSecret;
use kith_dataplane::{
    Dataplane, DataplaneHandle, DropCounters, Endpoint, ExternalMember, ExternalPeer, PeerSpec,
    Stats,
};
use kith_identity::{
    Bootstrap, BootstrapAddr, DeviceIdentity, Hostname, InviteToken, MAX_BOOTSTRAP,
};
use kith_ipc::{Event, NetworkSummary, PeerInfo, Problem, Request, Response, Status, Warning};
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
use crate::external::{ClientConfig, ExternalPeers, render_client};
use crate::ipc;
use crate::network::{Change, NetworkRuntime, NetworkSet};
use crate::os::{DynSystem, OsPlanner, PlanInputs};
use crate::server::{Listeners, ServerMode, stun_reply};
use crate::state::StateDir;
use crate::stealth::StealthOptions;
use crate::types::{
    Action, Chain, ChannelEvent, Command, ConnectivityEvent, NodeError, WriteThrottle,
};

/// Most nodes one gossip message may carry; the control channel caps it at 256.
const MAX_GOSSIP_NODES: usize = 256;
/// How often changed peer endpoints are written to disk at most.
const ENDPOINT_SAVE_INTERVAL: core::time::Duration = core::time::Duration::from_secs(30);
/// How often every mesh peer is sent a keepalive, so silence means it is gone.
const MESH_KEEPALIVE_SECS: u16 = 10;
/// How long a mesh peer may stay silent and still count as online: two missed keepalives.
const MESH_SILENCE: core::time::Duration = core::time::Duration::from_secs(25);

/// How long a plain WireGuard member, which sends a keepalive every 25s, may stay silent
/// and still count as online.
const CLIENT_SILENCE: core::time::Duration = core::time::Duration::from_secs(60);
/// How often traffic counters are read: to keep checking the paths of peers that are
/// exchanging packets, and for a gateway to see whether its phones are online.
const PEER_TICK: core::time::Duration = core::time::Duration::from_secs(2);

/// The one rule for whether a peer is online: something authenticated arrived from it
/// within `silence`.
fn is_online(heard_ago: Option<core::time::Duration>, silence: core::time::Duration) -> bool {
    heard_ago.is_some_and(|age| age < silence)
}
/// How often `public_hostname` is resolved again, to follow an address that changes.
const PUBLIC_HOSTNAME_REFRESH: core::time::Duration = core::time::Duration::from_secs(300);
/// Warnings kept for `kith status`.
const MAX_WARNINGS: usize = 20;
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
            dataplane: started.dataplane,
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
    dataplane: DataplaneHandle,
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

    /// Drops all direct UDP in and out while `blocked`, leaving relays as the only path.
    #[cfg(feature = "test-support")]
    pub async fn block_direct_udp(&self, blocked: bool) -> Result<(), NodeError> {
        Ok(self.dataplane.block_direct_udp(blocked).await?)
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
    /// The dataplane, for fault injection.
    #[cfg(feature = "test-support")]
    pub(crate) dataplane: DataplaneHandle,
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
    /// When each peer was last heard from, in Unix seconds, as of the last stats read.
    last_seen: BTreeMap<NodeId, u64>,
    presence: Presences,
    public_hostname: Option<String>,
    notes: Notes,
    paths: BTreeMap<NodeId, Endpoint>,
    gossiped: BTreeMap<NetworkId, Vec<(NodeId, Vec<SocketAddr>)>>,
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
            if let Err(error) = Box::pin(dataplane.run()).await {
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
            Err(error) => {
                tracing::warn!(network = %id, error = %Chain(&error), "skipping stored network");
            }
        }
    }
    Ok(networks)
}

/// Our mesh addresses and the IPv4 resolver, the only one served. Every node holding the
/// shared IPv6 resolver made kernels send from it, and peers drop what claims to come from
/// their own address.
fn local_addresses(map: &NetMap) -> Vec<(IpAddr, u8)> {
    let mut addresses = Vec::new();
    if let Some(local) = map.local_addresses() {
        addresses.push((IpAddr::V4(local.v4), map.ipv4_range().prefix_len()));
        addresses.push((IpAddr::V6(local.v6), map.ipv6_range().prefix_len()));
    }
    for resolver in map.resolver().into_iter().filter(IpAddr::is_ipv4) {
        if !addresses.iter().any(|(address, _)| *address == resolver) {
            addresses.push((resolver, 32));
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
        kith_transport_udp::set_socket_mark(kith_sys::socket_mark(Platform::current()?));
        let state = if options.memory_state {
            StateDir::in_memory()
        } else {
            StateDir::open(&config.daemon.state_dir)?
        };
        let identity = Arc::new(state.load_or_create_identity()?);
        let local = identity.node_id();
        let networks = load_networks(&state, local)?;
        let (link, channel) = bind_sockets(
            &identity,
            &networks,
            SocketAddr::new(options.listen_ip, config.network.listen_port),
            config.server.enabled && config.server.relay,
            &commands,
        )
        .await?;
        let bound = link.local_addr();
        let dataplanes = start_dataplane(&identity, link, options.tun, config)?;
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
        let server =
            start_server(config, &identity, relay_listen, &connectivity, &commands).await?;
        let external = restore_external(&state, config).await?;
        let endpoints = state.load_endpoints()?.into_iter().collect();
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
            dataplane: dataplanes.handle.clone(),
        };
        let mut daemon = Self {
            identity,
            last_seen: state.load_last_seen()?,
            presence: Presences::new(),
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
            endpoints,
            public_hostname: config.server.public_hostname.clone(),
            notes: Notes::default(),
            paths: BTreeMap::new(),
            gossiped: BTreeMap::new(),
            exit: None,
            ipc_task,
        };
        daemon.refresh(None).await?;
        daemon.restore_exit().await?;
        daemon.advertise_server().await;
        Ok((daemon, started))
    }

    /// Runs the loop until shutdown or until every command sender is gone.
    pub(crate) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
    ) -> Result<(), NodeError> {
        loop {
            self.save_endpoints_if_due(false);
            self.save_last_seen_if_due(false).await;
            self.tick_peers_if_due().await;
            let deadline = earliest(self.connectivity.poll_timeout(), self.peer_tick_deadline());
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown { reply }) => {
                        let result = self.shutdown().await;
                        let failed = result.is_err();
                        let _sent = reply.send(result);
                        return if failed { Err(NodeError::Stopped) } else { Ok(()) };
                    }
                    Some(command) => self.handle(command).await,
                    None => return self.shutdown().await,
                },
                event = self.dataplane_events.recv() => {
                    let Some(event) = event else {
                        self.shutdown().await?;
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
                let _sent = reply.send(self.shutdown().await);
            }
        }
        let flushed = self.flush_outputs().await;
        self.report(flushed);
    }

    fn report(&mut self, result: Result<(), NodeError>) {
        if let Err(error) = result {
            tracing::warn!(error = %Chain(&error), "node event failed");
            if self.notes.warnings.len() >= MAX_WARNINGS {
                self.notes.warnings.pop_front();
            }
            self.notes
                .warnings
                .push_back((Instant::now(), Chain(&error).to_string()));
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
            Action::RemoveDevice { network, node } => self.remove_device(network, node).await,
            Action::AdvertiseRelay {
                network,
                name,
                address,
                node,
            } => {
                let node = node.unwrap_or_else(|| identity.node_id());
                let relay = Relay::new(name, address.clone(), node)
                    .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
                self.write_admin_record(network, &Record::Relay(relay), true)
                    .await?;
                Ok(Response::RelayAdvertised { address })
            }
            Action::RemoveRelay { network, name } => {
                let relay = self
                    .member_map(network)?
                    .relays()
                    .iter()
                    .find(|relay| relay.name() == name)
                    .cloned()
                    .ok_or_else(|| NodeError::NotFound(format!("relay {name}")))?;
                self.write_admin_record(network, &Record::Relay(relay), false)
                    .await?;
                Ok(Response::RelayRemoved)
            }
            Action::SetExitGrant {
                network,
                node,
                allowed,
            } => {
                self.write_admin_record(network, &Record::Exit(node), allowed)
                    .await?;
                Ok(Response::ExitGrantSet)
            }
            Action::ServerMode { exit, relay } => self.server_mode(exit, relay).await,
            Action::ImportWireGuard { name, config } => {
                let network = self.external.import(&self.state, name, *config).await?;
                self.refresh(None).await?;
                Ok(Response::Imported { network })
            }
            Action::ExportWireGuard {
                network,
                device_name,
                tags,
            } => self.export(network, device_name, tags).await,
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
        self.advertise_server().await;
        Ok(Response::NetworkCreated { network })
    }

    async fn create_invite(
        &mut self,
        network: NetworkId,
        expires_in: core::time::Duration,
        max_uses: u16,
    ) -> Result<Response, NodeError> {
        self.member_map(network)?;
        if !self.is_admin(network)? {
            return Err(NodeError::InvalidArgument(
                "only an admin of this network can create invites; ask an admin to run `kith invite`"
                    .to_owned(),
            ));
        }
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
            let addrs = match entry.addr() {
                BootstrapAddr::Socket(addr) => vec![*addr],
                BootstrapAddr::Host(host, port) => resolve(host.as_str(), *port).await,
            };
            let known = self.endpoints.entry(entry.node()).or_default();
            for addr in addrs {
                if !known.contains(&addr) {
                    known.push(addr);
                }
            }
        }
        self.state.save_endpoints(&self.endpoints)?;
        self.persist(network)?;
        self.refresh(Some(network)).await?;
        self.advertise_server().await;
        let device = self
            .networks
            .maps()
            .get(&network)
            .and_then(NetMap::local_device)
            .map_or_else(|| device_name.to_owned(), |device| device.label.clone());
        Ok(Response::Joined { network, device })
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
            self.state.save_exit(None)?;
        }
        self.refresh(None).await?;
        Ok(Response::Left)
    }

    async fn remove_device(
        &mut self,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Response, NodeError> {
        if node == self.identity.node_id() {
            return Err(NodeError::InvalidArgument(
                "to take this device out of the network, run `kith leave`".to_owned(),
            ));
        }
        let member = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| ledger.view().is_member(&node))?;
        if !member {
            return Err(NodeError::NotFound(format!(
                "{node} is not in this network"
            )));
        }
        if !self.is_admin(network)? {
            return Err(NodeError::InvalidArgument(
                "only an admin of this network can remove devices".to_owned(),
            ));
        }
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?
            .remove_device(&identity, node, now())?;
        if self.exit == Some((network, node)) {
            self.exit = None;
            self.dataplane.set_exit(None).await?;
            self.state.save_exit(None)?;
        }
        self.commit(network).await?;
        Ok(Response::DeviceRemoved)
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
        self.state.save_exit(self.exit)?;
        self.refresh(None).await?;
        Ok(Response::ExitSet)
    }

    async fn restore_exit(&mut self) -> Result<(), NodeError> {
        let Some((network, node)) = self.state.load_exit()? else {
            return Ok(());
        };
        let granted = self
            .networks
            .maps()
            .get(&network)
            .is_some_and(|map| map.exit_nodes().contains(&node));
        if !granted {
            tracing::warn!(%network, %node, "saved exit node is no longer an exit; not using it");
            return self.state.save_exit(None);
        }
        self.exit = Some((network, node));
        self.dataplane.set_exit(self.exit).await?;
        self.refresh(None).await
    }

    fn is_admin(&self, network: NetworkId) -> Result<bool, NodeError> {
        let local = self.identity.node_id();
        self.networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| ledger.view().admins().contains(&local))
    }

    async fn write_admin_record(
        &mut self,
        network: NetworkId,
        record: &Record,
        present: bool,
    ) -> Result<(), NodeError> {
        if !self.is_admin(network)? {
            return Err(NodeError::InvalidArgument(format!(
                "this device is not an admin of {network}"
            )));
        }
        let value = if present {
            record
                .encode()
                .map_err(|error| NodeError::InvalidArgument(error.to_string()))?
        } else {
            Vec::new()
        };
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?
            .set_record(&identity, record.key(), value, now())?;
        self.commit(network).await
    }

    fn relay_address(&self) -> Option<String> {
        let bound = self.server.as_ref().and_then(ServerMode::relay_addr)?;
        if let Some(host) = &self.public_hostname {
            return Some(format!("{host}:{}", bound.port()));
        }
        let public = self
            .connectivity
            .local_endpoints()
            .into_iter()
            .find(|addr| is_public(addr.ip()))
            .map(|addr| SocketAddr::new(addr.ip(), bound.port()));
        let fallback = (!bound.ip().is_unspecified()).then_some(bound);
        public.or(fallback).map(|addr| addr.to_string())
    }

    async fn advertise_server(&mut self) {
        let Some((exit, relay)) = self
            .server
            .as_ref()
            .map(|server| (server.is_exit(), server.relay_addr().is_some()))
        else {
            return;
        };
        match self.advertise_roles(exit, relay).await {
            Ok(notes) => notes.iter().for_each(|note| tracing::info!("{note}")),
            Err(error) => {
                tracing::warn!(error = %Chain(&error), "could not advertise server roles");
            }
        }
    }

    async fn advertise_roles(&mut self, exit: bool, relay: bool) -> Result<Vec<String>, NodeError> {
        let local = self.identity.node_id();
        let address = if relay { self.relay_address() } else { None };
        let mut notes = Vec::new();
        if relay && address.is_none() {
            notes.push(
                "no public address is known yet; advertise the relay with `kith relay advertise`"
                    .to_owned(),
            );
        }
        let networks: Vec<(NetworkId, String, String, bool, bool)> =
            member_maps(self.networks.maps())
                .map(|map| {
                    let name = map.local_device().map_or_else(
                        || "server".to_owned(),
                        |device| device.name.to_ascii_lowercase(),
                    );
                    let advertised = map.relays().iter().any(|known| {
                        known.node() == local && Some(known.address()) == address.as_deref()
                    });
                    let granted = map.exit_nodes().contains(&local);
                    (
                        map.network_id(),
                        map.name().to_owned(),
                        name,
                        advertised,
                        granted,
                    )
                })
                .collect();
        for (network, network_name, device, advertised, granted) in networks {
            let admin = self.is_admin(network)?;
            if let Some(address) = address.as_ref().filter(|_| !advertised) {
                if admin {
                    let relay = Relay::new(device.clone(), address.clone(), local)
                        .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
                    self.write_admin_record(network, &Record::Relay(relay), true)
                        .await?;
                    notes.push(format!(
                        "advertised relay {device} at {address} in {network_name}"
                    ));
                } else {
                    notes.push(format!(
                        "not an admin of {network_name}; an admin must run: kith relay advertise {network_name} {device} {address} --node {local}"
                    ));
                }
            }
            if exit && !granted {
                if admin {
                    self.write_admin_record(network, &Record::Exit(local), true)
                        .await?;
                    notes.push(format!("granted exit in {network_name}"));
                } else {
                    notes.push(format!(
                        "not an admin of {network_name}; an admin must run: kith exit allow {network_name} {local}"
                    ));
                }
            }
        }
        Ok(notes)
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
        let notes = self.advertise_roles(exit, relay).await?;
        Ok(Response::ServerModeSet { notes })
    }

    /// Adds a plain WireGuard device, such as a phone, to `network` through this device,
    /// and returns its configs. Its private key exists only in them. The configs are
    /// written before the device is admitted, so a failure leaves no member behind.
    async fn export(
        &mut self,
        network: NetworkId,
        device_name: String,
        tags: Vec<String>,
    ) -> Result<Response, NodeError> {
        if !self.is_admin(network)? {
            return Err(NodeError::InvalidArgument(
                "only an admin of this network can add a device".to_owned(),
            ));
        }
        let gateway = self.external.gateway_public_key(&self.state)?;
        let secret = AgreementSecret::generate()
            .map_err(|error| NodeError::Io(std::io::Error::other(error)))?;
        let key = secret.public_key().to_bytes();
        let runtime = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?;
        let addresses = runtime.client_addresses(&key)?;
        let endpoint = self.bootstrap().first().map_or_else(
            || self.channel.local_addr().to_string(),
            |b| b.addr().to_string(),
        );
        let map = self.member_map(network)?;
        let invalid =
            |error: kith_types::PrefixError| NodeError::InvalidArgument(error.to_string());
        let mesh_ranges = [
            IpPrefix::V4(map.ipv4_range()),
            IpPrefix::V6(map.ipv6_range()),
        ];
        let everything_ranges = [
            IpPrefix::V4(Ipv4Prefix::new(Ipv4Addr::UNSPECIFIED, 0).map_err(invalid)?),
            IpPrefix::V6(Ipv6Prefix::new(Ipv6Addr::UNSPECIFIED, 0).map_err(invalid)?),
        ];
        let config = |allowed_ips: &[IpPrefix]| {
            render_client(&ClientConfig {
                secret: &secret,
                addresses,
                dns: map.resolver()[0],
                gateway,
                endpoint: &endpoint,
                allowed_ips,
            })
        };
        let mesh = config(&mesh_ranges)?;
        let everything = if self.server.as_ref().is_some_and(ServerMode::is_exit) {
            Some(config(&everything_ranges)?)
        } else {
            None
        };
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?.add_client(
            &identity,
            key,
            (device_name, tags),
            addresses,
            now(),
        )?;
        self.commit(network).await?;
        let id = kith_ledger::client_id(&key);
        let map = self.member_map(network)?;
        let Some(client) = map.clients().get(&id) else {
            return Err(NodeError::InvalidArgument(format!(
                "another device was given the same address at the same moment; remove this one with `kith remove {} {id}` and add it again",
                map.name()
            )));
        };
        Ok(Response::WireGuardConfig {
            device: client.peer.label.clone(),
            endpoint,
            mesh,
            everything,
        })
    }

    /// Our endpoints, minus any inside a mesh range: those exist only through the tunnel,
    /// however they were learned (an interface, STUN or a peer's view of us).
    fn underlay_endpoints(&self) -> Vec<SocketAddr> {
        let ranges: Vec<IpPrefix> = member_maps(self.networks.maps())
            .flat_map(|map| {
                [
                    IpPrefix::V4(map.ipv4_range()),
                    IpPrefix::V6(map.ipv6_range()),
                ]
            })
            .collect();
        outside(&ranges, self.connectivity.local_endpoints())
    }

    /// Where joiners reach us: `public_hostname` first, then public, then private addresses.
    fn bootstrap(&self) -> Vec<Bootstrap> {
        let control = self.channel.local_addr();
        let port = control.port();
        let mut sockets: Vec<SocketAddr> = if control.ip().is_unspecified() {
            self.underlay_endpoints()
                .into_iter()
                .map(|addr| SocketAddr::new(addr.ip(), port))
                .collect()
        } else {
            vec![control]
        };
        sockets.sort_by_key(|addr| !is_public(addr.ip()));
        let public =
            self.public_hostname
                .as_deref()
                .and_then(|host| match host.parse::<IpAddr>() {
                    Ok(ip) => Some(BootstrapAddr::Socket(SocketAddr::new(ip, port))),
                    Err(_) => Hostname::new(host)
                        .ok()
                        .map(|name| BootstrapAddr::Host(name, port)),
                });
        let node = self.identity.node_id();
        let mut bootstrap: Vec<Bootstrap> = Vec::new();
        for addr in public
            .into_iter()
            .chain(sockets.into_iter().map(BootstrapAddr::Socket))
        {
            let entry = Bootstrap::new(node, addr);
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
                    self.peer_infos(*id, map, &stats)
                        .iter()
                        .filter(|peer| peer.online),
                ),
                peers_total: count(
                    map.peers()
                        .iter()
                        .chain(map.clients().iter().map(|(id, c)| (id, &c.peer))),
                ),
                relays: count(map.relays().iter()),
                problems: self.problems(*id, map),
            })
            .collect();
        let now = Instant::now();
        Ok(Response::Status(Status {
            node: Some(self.identity.node_id()),
            networks,
            exit: self.server.as_ref().is_some_and(ServerMode::is_exit),
            relay: self
                .server
                .as_ref()
                .is_some_and(|server| server.relay_addr().is_some()),
            exit_node: self.exit.and_then(|(network, node)| {
                let map = self.networks.maps().get(&network)?;
                Some(format!("{}/{}", map.name(), map.peers().get(&node)?.label))
            }),
            listen_port: self.channel.local_addr().port(),
            dns_upstreams: self
                .notes
                .dns_forwarding
                .iter()
                .map(ToString::to_string)
                .collect(),
            drops: drop_reasons(&stats.drops),
            route_conflicts: self
                .os
                .conflicts()
                .iter()
                .map(ToString::to_string)
                .collect(),
            warnings: self
                .notes
                .warnings
                .iter()
                .map(|(at, message)| Warning {
                    secs_ago: now.saturating_duration_since(*at).as_secs(),
                    message: message.clone(),
                })
                .collect(),
        }))
    }

    async fn peers(&self, network: NetworkId) -> Result<Response, NodeError> {
        let map = self.member_map(network)?;
        let stats = self.dataplane.stats().await?;
        Ok(Response::Peers(self.peer_infos(network, map, &stats)))
    }

    /// What is wrong with `network` that its admins must fix, in words.
    fn problems(&self, network: NetworkId, map: &NetMap) -> Vec<Problem> {
        let name_of = |node: &NodeId| {
            self.networks
                .get(&network)
                .and_then(|runtime| {
                    runtime
                        .with_ledger(|ledger| {
                            ledger.view().devices().get(node).map(|d| d.name.clone())
                        })
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| node.short().to_string())
        };
        let remove = |node: &NodeId| format!("kith remove {} {node}", map.name());
        map.warnings()
            .iter()
            .filter_map(|warning| match warning {
                MapWarning::ClientAddressTaken { client } => Some(Problem {
                    what: format!(
                        "{} has no address: another device was given the same one at the same moment",
                        name_of(client)
                    ),
                    fix: format!("remove it with `{}` and add it again", remove(client)),
                }),
                MapWarning::ClientWithoutGateway { client, gateway } => Some(Problem {
                    what: format!(
                        "{} is cut off: {}, the device it connects through, was removed",
                        name_of(client),
                        name_of(gateway)
                    ),
                    fix: format!(
                        "add that device back, or remove it with `{}` and add it again through another",
                        remove(client)
                    ),
                }),
                MapWarning::InvalidPolicy(_)
                | MapWarning::InvalidTag { .. }
                | MapWarning::InvalidRecord { .. }
                | MapWarning::RecordForNonMember { .. }
                | _ => None,
            })
            .collect()
    }

    /// Every peer of `network` as `kith status` shows it, kith devices first, then plain
    /// WireGuard members. Liveness is decided here, once, by [`is_online`] from what this
    /// run of the dataplane heard; a member behind another gateway is online when that
    /// gateway is and says it is. The saved last-seen time only fills in when a peer was
    /// last heard from before a restart.
    fn peer_infos(&self, network: NetworkId, map: &NetMap, stats: &Stats) -> Vec<PeerInfo> {
        let at = wall_instant();
        let age = |when: std::time::Instant| at.saturating_duration_since(when);
        let mut infos: Vec<PeerInfo> = map
            .peers()
            .iter()
            .map(|(node, peer)| {
                let stat = stats
                    .peers
                    .iter()
                    .find(|stat| stat.network == network && stat.node == *node);
                let heard_ago = stat.and_then(|stat| stat.last_received).map(age);
                let online = is_online(heard_ago, MESH_SILENCE);
                let seen_ago = heard_ago.or_else(|| self.last_seen.get(node).map(|s| secs_ago(*s)));
                PeerInfo {
                    node: *node,
                    name: peer.label.clone(),
                    address: peer.addresses.v4.to_string(),
                    path: self.connectivity.path_kind(node).filter(|_| online),
                    path_reason: self.connectivity.path_reason(node).filter(|_| online),
                    last_seen_secs: seen_ago.map(|age| age.as_secs()),
                    endpoint: stat.and_then(|stat| stat.endpoint).filter(|_| online).map(
                        |endpoint| match endpoint {
                            Endpoint::Direct(addr) => addr.to_string(),
                            Endpoint::Relay => String::from("relay"),
                        },
                    ),
                    rx_bytes: stat.map_or(0, |stat| stat.rx_bytes),
                    tx_bytes: stat.map_or(0, |stat| stat.tx_bytes),
                    online,
                    display_name: peer.name.clone(),
                    gateway: None,
                }
            })
            .collect();
        let local = self.identity.node_id();
        let label_of = |node: &NodeId| {
            map.peers()
                .get(node)
                .or_else(|| map.local_device().filter(|_| *node == local))
                .map(|peer| peer.label.clone())
        };
        for (id, client) in map.clients() {
            let (online, seen_ago, endpoint, rx_bytes, tx_bytes) = if client.gateway == local {
                let stat = stats
                    .external
                    .iter()
                    .find(|stat| stat.public_key == client.key);
                let heard_ago = stat.and_then(|stat| stat.last_received).map(age);
                let online = is_online(heard_ago, CLIENT_SILENCE);
                (
                    online,
                    heard_ago.or_else(|| self.last_seen.get(id).map(|s| secs_ago(*s))),
                    stat.and_then(|stat| stat.endpoint)
                        .filter(|_| online)
                        .map(|e| e.to_string()),
                    stat.map_or(0, |stat| stat.rx_bytes),
                    stat.map_or(0, |stat| stat.tx_bytes),
                )
            } else {
                let gateway_online = infos
                    .iter()
                    .any(|info| info.node == client.gateway && info.online);
                let said = self
                    .presence
                    .said
                    .get(&(network, *id))
                    .filter(|(from, _)| *from == client.gateway)
                    .map(|(_, presence)| *presence);
                let online = gateway_online && said.is_some_and(|presence| presence.online);
                let seen_ago = said
                    .filter(|presence| presence.last_seen > 0)
                    .map(|presence| secs_ago(presence.last_seen));
                (online, seen_ago, None, 0, 0)
            };
            infos.push(PeerInfo {
                node: *id,
                name: client.peer.label.clone(),
                address: client.peer.addresses.v4.to_string(),
                path: None,
                path_reason: None,
                last_seen_secs: seen_ago.map(|age| age.as_secs()),
                endpoint,
                rx_bytes,
                tx_bytes,
                online,
                display_name: client.peer.name.clone(),
                gateway: label_of(&client.gateway),
            });
        }
        infos
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
            ChannelEvent::Presence {
                network,
                from,
                entries,
            } => {
                let Some(map) = self.networks.maps().get(&network) else {
                    return Ok(());
                };
                self.presence
                    .said
                    .retain(|(id, _), (gateway, _)| *id != network || *gateway != from);
                for (client, presence) in entries {
                    if map
                        .clients()
                        .get(&client)
                        .is_some_and(|c| c.gateway == from)
                    {
                        self.presence
                            .said
                            .insert((network, client), (from, presence));
                    }
                }
                Ok(())
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
                    self.notes.endpoint_writes.mark();
                    self.gossip()?;
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
                self.connectivity.note_activity(peer, at);
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

    /// Resolves `public_hostname` again when it is due; a server whose public address
    /// changes on restart, behind a 1:1 NAT as on EC2, offers it as its first endpoint.
    /// Also resolves relays advertised by hostname, so STUN can use them.
    async fn refresh_public_endpoints(&mut self) {
        let now = Instant::now();
        let relays: BTreeSet<String> = relays(self.networks.maps())
            .iter()
            .map(|relay| relay.address().to_owned())
            .filter(|address| address.parse::<SocketAddr>().is_err())
            .collect();
        let fresh = self
            .notes
            .public_resolved
            .is_some_and(|at| now.saturating_duration_since(at) < PUBLIC_HOSTNAME_REFRESH);
        if fresh
            && relays
                .iter()
                .all(|r| self.notes.relay_addresses.contains_key(r))
        {
            return;
        }
        let mut relay_addresses = BTreeMap::new();
        for address in relays {
            let resolved = match address.rsplit_once(':') {
                Some((host, port)) => match port.parse() {
                    Ok(port) => resolve(host, port).await,
                    Err(_) => Vec::new(),
                },
                None => Vec::new(),
            };
            relay_addresses.insert(address, resolved);
        }
        self.notes.relay_addresses = relay_addresses;
        self.notes.public_resolved = Some(now);
        if let Some(host) = self.public_hostname.clone() {
            let found = resolve(&host, self.channel.local_addr().port()).await;
            if found.is_empty() {
                tracing::warn!(%host, "public_hostname does not resolve; peers outside this network cannot reach us directly");
            }
            self.connectivity.set_configured_endpoints(found);
        }
    }

    async fn refresh(&mut self, changed: Option<NetworkId>) -> Result<(), NodeError> {
        self.refresh_public_endpoints().await;
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
        self.connectivity.set_mesh_ranges(
            member_maps(maps)
                .flat_map(|map| {
                    [
                        IpPrefix::V4(map.ipv4_range()),
                        IpPrefix::V6(map.ipv6_range()),
                    ]
                })
                .collect(),
        );
        self.connectivity.set_peers(&peers, at);
        let own_relay = self
            .server
            .as_ref()
            .and_then(ServerMode::relay_addr)
            .map(loopback);
        self.connectivity
            .set_relays(&relays, own_relay, &self.notes.relay_addresses)?;
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
        let maps = self.networks.maps();
        self.presence
            .said
            .retain(|(network, client), (gateway, _)| {
                maps.get(network)
                    .and_then(|map| map.clients().get(client))
                    .is_some_and(|client| client.gateway == *gateway)
            });
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
        let upstreams = if upstreams.is_empty() {
            self.os.system_resolvers()
        } else {
            upstreams.clone()
        };
        if upstreams.is_empty() {
            tracing::warn!(
                "no upstream DNS servers found; names outside the mesh will not resolve; set dns.fallback_upstreams"
            );
        }
        tracing::debug!(?upstreams, "DNS upstreams");
        self.notes.dns_forwarding.clone_from(&upstreams);
        match DnsService::start(listen, upstreams).await {
            Ok(dns) => {
                tracing::debug!(%listen, "DNS listening");
                self.dns = Some(dns);
            }
            Err(error) => {
                tracing::warn!(%listen, error = %Chain(&error), "DNS server did not start");
            }
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
                allowed_ips.extend(
                    client_prefixes(map)?
                        .into_iter()
                        .filter(|(_, gateway)| gateway == node)
                        .map(|(prefix, _)| prefix),
                );
                specs.push(PeerSpec {
                    network: *id,
                    node: *node,
                    allowed_ips,
                    endpoint: self.paths.get(node).copied(),
                    persistent_keepalive: Some(MESH_KEEPALIVE_SECS),
                });
            }
        }
        Ok(self.dataplane.set_peers(specs).await?)
    }

    async fn push_external(&mut self) -> Result<(), NodeError> {
        let interfaces = self.external.interfaces(self.local_clients()?);
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

    /// The plain WireGuard members this device is the gateway for, in every network.
    fn local_clients(&self) -> Result<Vec<ExternalPeer>, NodeError> {
        let local = self.identity.node_id();
        let mut peers: Vec<ExternalPeer> = Vec::new();
        for (network, map) in member_maps_with_ids(self.networks.maps()) {
            for (id, client) in map.clients().iter().filter(|(_, c)| c.gateway == local) {
                if peers.iter().any(|peer| peer.public_key == client.key) {
                    tracing::warn!(client = %id.short(), "a WireGuard key is in two networks; serving it in the first");
                    continue;
                }
                peers.push(ExternalPeer {
                    public_key: client.key,
                    preshared_key: None,
                    endpoint: None,
                    allowed_ips: host_prefixes(client.peer.addresses)?.to_vec(),
                    persistent_keepalive: None,
                    member: Some(ExternalMember {
                        network,
                        policy: client.filter.clone().unwrap_or_default(),
                        resolver: map.resolver()[0],
                    }),
                });
            }
        }
        Ok(peers)
    }

    fn apply_os(&mut self) -> Result<(), NodeError> {
        let maps = self.networks.maps();
        let exit_endpoints = self.exit_bypass(maps);
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

    /// Underlay addresses that must keep the original route while an exit is in use: every
    /// known address of the exit node and of every relay. Without them, on systems with no
    /// fwmark, kith's own traffic to them would enter the tunnel it carries.
    fn exit_bypass(&self, maps: &Networks) -> Vec<IpAddr> {
        let Some((_, node)) = self.exit else {
            return Vec::new();
        };
        let current = self.paths.get(&node).and_then(|endpoint| match endpoint {
            Endpoint::Direct(addr) => Some(*addr),
            Endpoint::Relay => None,
        });
        let relays = relays(maps);
        let relay_addrs =
            relays
                .iter()
                .flat_map(|relay| match relay.address().parse::<SocketAddr>() {
                    Ok(addr) => vec![addr],
                    Err(_) => self
                        .notes
                        .relay_addresses
                        .get(relay.address())
                        .cloned()
                        .unwrap_or_default(),
                });
        let ranges: Vec<IpPrefix> = member_maps(maps)
            .flat_map(|map| {
                [
                    IpPrefix::V4(map.ipv4_range()),
                    IpPrefix::V6(map.ipv6_range()),
                ]
            })
            .collect();
        let mut ips: Vec<IpAddr> = outside(
            &ranges,
            self.endpoints
                .get(&node)
                .into_iter()
                .flatten()
                .copied()
                .chain(current)
                .chain(relay_addrs)
                .collect(),
        )
        .into_iter()
        .map(|addr| addr.ip())
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
        .collect();
        ips.sort();
        ips.dedup();
        ips
    }

    fn update_targets(&mut self) -> Result<(), NodeError> {
        let maps = self.networks.maps();
        let members: Vec<&NetMap> = member_maps(maps).collect();
        let targets = pick_targets(&members, &relays(maps), &self.endpoints);
        self.channel.set_targets(targets)
    }

    /// Shares our endpoints and every member's last known ones, so members who never met
    /// directly can still find each other.
    fn gossip(&mut self) -> Result<(), NodeError> {
        let local = self.identity.node_id();
        let own = self.underlay_endpoints();
        for runtime in self.networks.iter() {
            let map = runtime.netmap();
            if !map.is_member() {
                continue;
            }
            let mut entries = vec![(local, own.clone())];
            entries.extend(
                self.endpoints
                    .iter()
                    .filter(|(node, addrs)| map.peers().contains_key(node) && !addrs.is_empty())
                    .map(|(node, addrs)| (*node, addrs.clone())),
            );
            entries.truncate(MAX_GOSSIP_NODES);
            debug_assert!(
                entries.iter().skip(1).all(|(node, _)| *node != local),
                "gossip lists this node twice"
            );
            if self.gossiped.get(&runtime.id()) == Some(&entries) {
                continue;
            }
            self.channel.gossip(runtime.id(), entries.clone())?;
            self.gossiped.insert(runtime.id(), entries);
        }
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

    /// Saves what is worth keeping and reverts the OS plan.
    pub(crate) async fn shutdown(&mut self) -> Result<(), NodeError> {
        self.save_endpoints_if_due(true);
        self.save_last_seen_if_due(true).await;
        self.os.revert()
    }

    /// Folds the dataplane's latest receive times into `last_seen`, keeping only peers
    /// still in some network, and writes it when it changed, at most every
    /// [`ENDPOINT_SAVE_INTERVAL`] or now with `force`.
    async fn save_last_seen_if_due(&mut self, force: bool) {
        let now = Instant::now();
        self.notes.last_seen_writes.mark();
        if !self.notes.last_seen_writes.due(now, force) {
            return;
        }
        let before = self.last_seen.clone();
        if let Ok(stats) = self.dataplane.stats().await {
            self.absorb_last_seen(&stats);
        }
        let known: BTreeSet<NodeId> = member_maps(self.networks.maps())
            .flat_map(|map| map.peers().keys().chain(map.clients().keys()).copied())
            .collect();
        self.last_seen.retain(|node, _| known.contains(node));
        if self.last_seen == before {
            self.notes.last_seen_writes.written(now);
            return;
        }
        match self.state.save_last_seen(&self.last_seen) {
            Ok(()) => self.notes.last_seen_writes.written(now),
            Err(error) => {
                tracing::warn!(error = %Chain(&error), "could not save when peers were last seen");
            }
        }
    }

    fn absorb_last_seen(&mut self, stats: &Stats) {
        let clients = self.local_client_ids();
        let heard = stats
            .peers
            .iter()
            .map(|stat| (Some(stat.node), stat.last_received))
            .chain(
                stats
                    .external
                    .iter()
                    .map(|stat| (clients.get(&stat.public_key).copied(), stat.last_received)),
            );
        for (node, at) in heard {
            if let (Some(node), Some(secs)) = (node, at.and_then(unix_secs)) {
                let seen = self.last_seen.entry(node).or_insert(0);
                *seen = (*seen).max(secs);
            }
        }
    }

    /// This device's plain WireGuard members, by WireGuard key.
    fn local_client_ids(&self) -> BTreeMap<[u8; 32], NodeId> {
        let local = self.identity.node_id();
        member_maps(self.networks.maps())
            .flat_map(|map| map.clients().iter())
            .filter(|(_, client)| client.gateway == local)
            .map(|(id, client)| (client.key, *id))
            .collect()
    }

    /// When to next read traffic counters: while there is any peer or phone to watch.
    fn peer_tick_deadline(&self) -> Option<Instant> {
        let watching = member_maps(self.networks.maps())
            .any(|map| !map.peers().is_empty() || !map.clients().is_empty());
        (watching || !self.presence.announced.is_empty()).then_some(self.presence.due)
    }

    /// Reads traffic counters every [`PEER_TICK`]: peers exchanging packets, keepalives
    /// included, keep having their paths checked, so a dead direct path falls back to the
    /// relay within seconds; and a gateway says whether its phones are online.
    async fn tick_peers_if_due(&mut self) {
        let now = Instant::now();
        if self.peer_tick_deadline().is_none_or(|due| due > now) {
            return;
        }
        self.presence.due = now + PEER_TICK;
        let Ok(stats) = self.dataplane.stats().await else {
            return;
        };
        for stat in &stats.peers {
            let counters = (stat.rx_bytes, stat.tx_bytes);
            if self.presence.traffic.insert(stat.node, counters) != Some(counters) {
                self.connectivity.note_activity(stat.node, now);
            }
        }
        let known: BTreeSet<NodeId> = stats.peers.iter().map(|stat| stat.node).collect();
        self.presence.traffic.retain(|node, _| known.contains(node));
        self.announce_presence(&stats);
    }

    /// Tells the network which of this device's plain WireGuard members are online,
    /// whenever that changes.
    fn announce_presence(&mut self, stats: &Stats) {
        let local = self.identity.node_id();
        let at = wall_instant();
        let mut wanted: BTreeMap<NetworkId, Vec<(NodeId, Presence)>> = BTreeMap::new();
        for (network, map) in member_maps_with_ids(self.networks.maps()) {
            for (id, client) in map.clients().iter().filter(|(_, c)| c.gateway == local) {
                let heard = stats
                    .external
                    .iter()
                    .find(|stat| stat.public_key == client.key)
                    .and_then(|stat| stat.last_received);
                let presence = Presence {
                    online: is_online(
                        heard.map(|when| at.saturating_duration_since(when)),
                        CLIENT_SILENCE,
                    ),
                    last_seen: heard
                        .and_then(unix_secs)
                        .or_else(|| self.last_seen.get(id).copied())
                        .unwrap_or(0),
                };
                wanted.entry(network).or_default().push((*id, presence));
            }
        }
        let networks: BTreeSet<NetworkId> = wanted
            .keys()
            .chain(self.presence.announced.keys())
            .copied()
            .collect();
        for network in networks {
            let entries = wanted.remove(&network).unwrap_or_default();
            let unchanged = self.presence.announced.get(&network).is_some_and(|before| {
                before.len() == entries.len()
                    && before.iter().zip(&entries).all(|((a, x), (b, y))| {
                        a == b && x.online == y.online && (x.online || x.last_seen == y.last_seen)
                    })
            });
            if unchanged {
                continue;
            }
            if let Err(error) = self.channel.announce_presence(network, entries.clone()) {
                tracing::warn!(error = %Chain(&error), "could not announce which phones are online");
                continue;
            }
            if entries.is_empty() {
                self.presence.announced.remove(&network);
            } else {
                self.presence.announced.insert(network, entries);
            }
        }
    }

    /// Writes peer endpoints at most every [`ENDPOINT_SAVE_INTERVAL`], or now with `force`.
    /// Each write syncs to disk, and behind some NATs endpoints change many times a second.
    fn save_endpoints_if_due(&mut self, force: bool) {
        let now = Instant::now();
        if !self.notes.endpoint_writes.due(now, force) {
            return;
        }
        match self.state.save_endpoints(&self.endpoints) {
            Ok(()) => self.notes.endpoint_writes.written(now),
            Err(error) => tracing::warn!(error = %Chain(&error), "could not save peer endpoints"),
        }
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

/// Binds the WireGuard socket and the control channel on the same port. With port 0 the OS
/// picks one, and we pick again until TCP (and a relay's ports, one above) are free too.
async fn bind_sockets(
    identity: &Arc<DeviceIdentity>,
    networks: &NetworkSet,
    listen: SocketAddr,
    relay: bool,
    commands: &mpsc::Sender<Command>,
) -> Result<(UdpLink, ChannelLayer), NodeError> {
    const ATTEMPTS: usize = 16;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let retry = listen.port() == 0 && attempt < ATTEMPTS;
        let link = match UdpLink::bind(listen) {
            Ok(link) => link,
            Err(kith_transport_udp::UdpError::Bind { source, .. })
                if source.kind() == std::io::ErrorKind::AddrInUse =>
            {
                return Err(NodeError::PortInUse {
                    port: listen.port(),
                    protocol: "UDP, WireGuard",
                });
            }
            Err(error) => return Err(error.into()),
        };
        let bound = link.local_addr();
        let channel = match ChannelLayer::start(
            Arc::clone(identity),
            networks.registry(),
            bound,
            commands.clone(),
        )
        .await
        {
            Ok(channel) => channel,
            Err(NodeError::Channel(kith_channel::ChannelError::Io(error)))
                if error.kind() == std::io::ErrorKind::AddrInUse =>
            {
                if retry {
                    continue;
                }
                return Err(NodeError::PortInUse {
                    port: bound.port(),
                    protocol: "TCP, control channel",
                });
            }
            Err(error) => return Err(error),
        };
        let relay_free = !relay || {
            let next = SocketAddr::new(bound.ip(), bound.port().saturating_add(1));
            std::net::TcpListener::bind(next).is_ok() && std::net::UdpSocket::bind(next).is_ok()
        };
        if relay_free {
            return Ok((link, channel));
        }
        if !retry {
            return Err(NodeError::PortInUse {
                port: bound.port().saturating_add(1),
                protocol: "TCP and UDP, relay and STUN",
            });
        }
    }
}

/// Bookkeeping for diagnostics and for writing peer endpoints sparingly.
#[derive(Debug)]
struct Notes {
    /// Recent warnings with when they happened, for `kith status` and `kith doctor`.
    warnings: VecDeque<(Instant, String)>,
    /// Where the DNS server forwards names outside the mesh.
    dns_forwarding: Vec<SocketAddr>,
    /// When to write changed peer endpoints.
    endpoint_writes: WriteThrottle,
    /// When to write when peers were last heard from.
    last_seen_writes: WriteThrottle,
    /// When `public_hostname` and relay hostnames last resolved.
    public_resolved: Option<Instant>,
    /// What each relay `host:port` resolved to, for STUN.
    relay_addresses: BTreeMap<String, Vec<SocketAddr>>,
}

impl Default for Notes {
    fn default() -> Self {
        Self {
            warnings: VecDeque::new(),
            dns_forwarding: Vec::new(),
            endpoint_writes: WriteThrottle::new(ENDPOINT_SAVE_INTERVAL),
            last_seen_writes: WriteThrottle::new(ENDPOINT_SAVE_INTERVAL),
            public_resolved: None,
            relay_addresses: BTreeMap::new(),
        }
    }
}

/// Starts server mode when the config enables it.
async fn start_server(
    config: &Config,
    identity: &DeviceIdentity,
    relay_listen: SocketAddr,
    connectivity: &Connectivity,
    commands: &mpsc::Sender<Command>,
) -> Result<Option<ServerMode>, NodeError> {
    if !config.server.enabled {
        return Ok(None);
    }
    let server = ServerMode::start(
        identity,
        Listeners::from_config(&config.server, relay_listen),
        config.server.exit,
        connectivity.quic_links(),
        commands.clone(),
    )
    .await?;
    Ok(Some(server))
}

/// What the periodic read of traffic counters keeps: whether plain WireGuard members,
/// such as phones, are online, and which peers are exchanging packets.
#[derive(Debug)]
struct Presences {
    /// Other gateways' members, as each gateway said: member to (gateway, presence).
    said: BTreeMap<(NetworkId, NodeId), (NodeId, Presence)>,
    /// What this device last said about its own members, per network.
    announced: BTreeMap<NetworkId, Vec<(NodeId, Presence)>>,
    /// Each peer's received and sent bytes at the last tick.
    traffic: BTreeMap<NodeId, (u64, u64)>,
    /// When to next read traffic counters.
    due: Instant,
}

impl Presences {
    fn new() -> Self {
        Self {
            said: BTreeMap::new(),
            announced: BTreeMap::new(),
            traffic: BTreeMap::new(),
            due: Instant::now(),
        }
    }
}

/// The addresses `host` resolves to, on `port`; an IP literal is itself.
async fn resolve(host: &str, port: u16) -> Vec<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return vec![SocketAddr::new(ip, port)];
    }
    match tokio::net::lookup_host((host, port)).await {
        Ok(found) => found.collect(),
        Err(error) => {
            tracing::debug!(%host, %error, "could not resolve");
            Vec::new()
        }
    }
}

/// Where this node reaches a server it runs bound on `addr`: loopback for an unspecified
/// bind, else `addr` itself.
fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::from((Ipv4Addr::LOCALHOST, addr.port()))
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::from((Ipv6Addr::LOCALHOST, addr.port()))
        }
        IpAddr::V4(_) | IpAddr::V6(_) => addr,
    }
}

/// The endpoints that lie outside every one of `ranges`.
fn outside(ranges: &[IpPrefix], endpoints: Vec<SocketAddr>) -> Vec<SocketAddr> {
    endpoints
        .into_iter()
        .filter(|addr| !ranges.iter().any(|range| range.contains(addr.ip())))
        .collect()
}

/// Nonzero drop counters by name.
fn drop_reasons(drops: &DropCounters) -> BTreeMap<String, u64> {
    [
        ("no_route", drops.no_route),
        ("no_endpoint", drops.no_endpoint),
        ("invalid", drops.invalid),
        ("source_not_allowed", drops.source_not_allowed),
        ("policy", drops.policy),
        ("malformed", drops.malformed),
        ("table_full", drops.table_full),
        ("overflow", drops.overflow),
        ("foreign_source", drops.foreign_source),
        ("after_exit", drops.after_exit),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(name, count)| (name.to_owned(), count))
    .collect()
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation())
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local())
        }
    }
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
    for (prefix, gateway) in client_prefixes(map)? {
        routes.push((prefix, gateway));
    }
    Ok(routes)
}

/// Every plain WireGuard member's addresses with the kith peer that reaches it. Members
/// behind the local device are left out: the gateway interface serves them.
fn client_prefixes(map: &NetMap) -> Result<Vec<(IpPrefix, NodeId)>, NodeError> {
    let mut prefixes = Vec::new();
    for client in map.clients().values() {
        if map.peers().contains_key(&client.gateway) {
            for prefix in host_prefixes(client.peer.addresses)? {
                prefixes.push((prefix, client.gateway));
            }
        }
    }
    Ok(prefixes)
}

fn host_prefixes(addresses: Addresses) -> Result<[IpPrefix; 2], NodeError> {
    let invalid = |error: kith_types::PrefixError| NodeError::InvalidArgument(error.to_string());
    Ok([
        IpPrefix::V4(Ipv4Prefix::new(addresses.v4, 32).map_err(invalid)?),
        IpPrefix::V6(Ipv6Prefix::new(addresses.v6, 128).map_err(invalid)?),
    ])
}

/// `at` as Unix seconds, via how long ago it was.
fn unix_secs(at: std::time::Instant) -> Option<u64> {
    let ago = wall_instant().saturating_duration_since(at);
    let now = now().as_millis() / 1000;
    now.checked_sub(ago.as_secs())
}

/// How long ago `secs` (Unix seconds) was; zero for the future, as after a clock change.
fn secs_ago(secs: u64) -> core::time::Duration {
    core::time::Duration::from_secs((now().as_millis() / 1000).saturating_sub(secs))
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

fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => core::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use core::net::SocketAddr;

    use kith_types::IpPrefix;

    use super::{outside, resolve};

    #[test]
    fn mesh_addresses_are_never_shared_as_endpoints() {
        let ranges: Vec<IpPrefix> = vec![
            "100.64.0.0/10".parse().unwrap(),
            "fd7a:115c:a1e0::/48".parse().unwrap(),
        ];
        let addr = |text: &str| text.parse::<SocketAddr>().unwrap();
        let learned = vec![
            addr("192.168.1.20:41641"),
            addr("100.108.151.234:41641"),
            addr("[fd7a:115c:a1e0:a2ff::1]:41641"),
            addr("203.0.113.7:61000"),
        ];
        assert_eq!(
            outside(&ranges, learned),
            [addr("192.168.1.20:41641"), addr("203.0.113.7:61000")]
        );
    }

    #[tokio::test]
    async fn public_hostnames_resolve_to_endpoints_on_the_wireguard_port() {
        let named = resolve("localhost", 41641).await;
        assert!(
            named.contains(&"127.0.0.1:41641".parse().unwrap()),
            "{named:?}"
        );
        assert_eq!(
            resolve("203.0.113.7", 41641).await,
            ["203.0.113.7:41641".parse().unwrap()]
        );
        assert_eq!(
            resolve("does-not-exist.invalid", 41641).await,
            Vec::<SocketAddr>::new()
        );
    }

    #[test]
    fn a_peer_is_online_only_while_it_is_heard_from() {
        use core::time::Duration;

        use super::{MESH_KEEPALIVE_SECS, MESH_SILENCE, is_online};
        let keepalive = Duration::from_secs(u64::from(MESH_KEEPALIVE_SECS));
        assert!(
            MESH_SILENCE > keepalive * 2,
            "one late keepalive must not flap"
        );
        assert!(is_online(Some(Duration::ZERO), MESH_SILENCE));
        assert!(is_online(Some(Duration::from_millis(24_999)), MESH_SILENCE));
        assert!(!is_online(Some(MESH_SILENCE), MESH_SILENCE));
        assert!(
            !is_online(None, MESH_SILENCE),
            "never heard from is offline"
        );
    }
}
