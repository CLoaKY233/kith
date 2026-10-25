#![expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::collections::BTreeMap;
use std::sync::Arc;

use kith_config::Config;
use kith_dataplane::DataplaneHandle;
use kith_identity::DeviceIdentity;
use kith_ipc::{Event, Request, Response};
use kith_sys::{OsSystem, System};
use kith_types::{NetworkId, NodeId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::channel::ChannelLayer;
use crate::connectivity::{Connectivity, Output};
use crate::dns::DnsService;
use crate::external::ExternalPeers;
use crate::ipc;
use crate::network::{Change, NetworkSet};
use crate::os::{DynSystem, OsPlanner};
use crate::server::ServerMode;
use crate::state::StateDir;
use crate::types::{Action, ChannelEvent, Command, ConnectivityEvent, NodeError};

const COMMAND_QUEUE: usize = 256;

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
    pub fn new(config: Config, system: impl System + Send + 'static) -> Self {
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
    pub async fn send_packet(&self, packet: Vec<u8>) -> Result<(), NodeError> {
        Err(NodeError::Pending("daemon: in-memory TUN send"))
    }

    /// Reads the next IP packet the node delivered to the in-memory TUN.
    pub async fn recv_packet(&self) -> Result<Vec<u8>, NodeError> {
        Err(NodeError::Pending("daemon: in-memory TUN receive"))
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
#[derive(Debug, Clone, Copy)]
pub(crate) struct Started {
    /// The DNS server, when enabled.
    pub(crate) dns_addr: Option<SocketAddr>,
    /// The control listener.
    pub(crate) control_addr: SocketAddr,
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
    os: OsPlanner,
    server: Option<ServerMode>,
    external: ExternalPeers,
    dataplane: DataplaneHandle,
    dataplane_events: mpsc::Receiver<kith_dataplane::Event>,
    events: broadcast::Sender<Event>,
    endpoints: BTreeMap<NodeId, Vec<SocketAddr>>,
    exit: Option<(NetworkId, NodeId)>,
}

impl Daemon {
    /// Loads state, creates the TUN and dataplane, and starts every subsystem.
    pub(crate) async fn build(
        options: NodeOptions,
        commands: mpsc::Sender<Command>,
    ) -> Result<(Self, Started), NodeError> {
        Err(NodeError::Pending("daemon: build"))
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
                let handled = self.on_connectivity(event).await;
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

    /// Performs a local action and builds its IPC response.
    pub(crate) async fn execute(&mut self, action: Action) -> Result<Response, NodeError> {
        Err(NodeError::Pending("daemon: execute"))
    }

    /// Handles ledger changes and endpoint gossip reported by the control channel.
    pub(crate) async fn on_channel(&mut self, event: ChannelEvent) -> Result<(), NodeError> {
        Err(NodeError::Pending("daemon: channel event"))
    }

    /// Handles relayed datagrams, host network changes and port mappings.
    pub(crate) async fn on_connectivity(
        &mut self,
        event: ConnectivityEvent,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("daemon: connectivity event"))
    }

    /// Routes dataplane events: STUN to the server or connectivity, relay sends, handshakes.
    pub(crate) async fn on_dataplane(
        &mut self,
        event: kith_dataplane::Event,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("daemon: dataplane event"))
    }

    /// Persists the ledger and pushes a map change to the dataplane, DNS, OS and channel.
    pub(crate) async fn apply_change(&mut self, change: Change) -> Result<(), NodeError> {
        Err(NodeError::Pending("daemon: apply change"))
    }

    /// Executes queued connectivity outputs against the dataplane.
    pub(crate) async fn flush_outputs(&mut self) -> Result<(), NodeError> {
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
                    return Err(NodeError::Pending("daemon: path change"));
                }
            }
        }
        Ok(())
    }

    /// Reverts the OS plan.
    pub(crate) fn shutdown(&mut self) -> Result<(), NodeError> {
        self.os.revert()
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => core::future::pending().await,
    }
}
