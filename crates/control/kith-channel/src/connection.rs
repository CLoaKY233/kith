use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::Duration;

use kith_ledger::{JoinProof, Ledger, SignedOp};
use kith_sync::SyncSession;
use kith_types::{NetworkId, NodeId};
use tokio::io::{AsyncWrite, AsyncWriteExt, ReadHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, timeout};
use tokio_rustls::TlsStream;
use tracing::{debug, warn};

use crate::channel::{HANDSHAKE_TIMEOUT, Hub};
use crate::message::{Entries, FrameReader, Message};
use crate::{ChannelError, ChannelHandler};

const RESYNC_INTERVAL: Duration = Duration::from_secs(60);

const IDLE_TIMEOUT: Duration = Duration::from_secs(150);

const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_UNADMITTED_PENDING: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) enum Command {
    Changed(NetworkId),
    Endpoints(NetworkId, Arc<[u8]>),
}

pub(crate) struct Peer {
    pub(crate) id: u64,
    pub(crate) node: NodeId,
    pub(crate) addr: SocketAddr,
}

pub(crate) fn shared_networks<H: ChannelHandler>(handler: &H, peer: NodeId) -> BTreeSet<NetworkId> {
    handler
        .networks()
        .into_iter()
        .filter(|network| handler.is_member(*network, peer))
        .collect()
}

pub(crate) async fn serve_accepted<H: ChannelHandler>(
    hub: Weak<Hub<H>>,
    handler: Arc<H>,
    peer: Peer,
    stream: TlsStream<TcpStream>,
) {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (connection, frames) = Connection::new(hub, handler, peer, Role::Acceptor, Some(commands));
    connection.run(stream, frames, receiver, None).await;
}

pub(crate) fn spawn_dialed<H: ChannelHandler>(
    hub: &Arc<Hub<H>>,
    peer: Peer,
    stream: TlsStream<TcpStream>,
    offered: BTreeSet<NetworkId>,
) -> oneshot::Sender<()> {
    let (commands, receiver) = mpsc::unbounded_channel();
    hub.register(&peer, commands);
    let (keep, kept) = oneshot::channel();
    let handler = Arc::clone(&hub.handler);
    let (mut connection, frames) =
        Connection::new(Arc::downgrade(hub), handler, peer, Role::Dialer, None);
    connection.offered = offered;
    tokio::spawn(connection.run(stream, frames, receiver, Some(kept)));
    keep
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Acceptor,
    Dialer,
}

enum Close {
    PeerClosed,
    Local,
    TimedOut,
    Rejected,
    NotMember,
    Violation(&'static str),
    Failed(ChannelError),
}

struct Slot {
    session: SyncSession,
    rerun: bool,
    changed: bool,
    seen_by_resync: bool,
}

impl Slot {
    const fn new(session: SyncSession) -> Self {
        Self {
            session,
            rerun: false,
            changed: false,
            seen_by_resync: false,
        }
    }
}

struct Connection<H> {
    hub: Weak<Hub<H>>,
    handler: Arc<H>,
    peer: Peer,
    role: Role,
    member: bool,
    unregistered: Option<mpsc::UnboundedSender<Command>>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    offered: BTreeSet<NetworkId>,
    peer_offered: BTreeSet<NetworkId>,
    active: BTreeSet<NetworkId>,
    sessions: BTreeMap<NetworkId, Slot>,
}

impl<H: ChannelHandler> Connection<H> {
    fn new(
        hub: Weak<Hub<H>>,
        handler: Arc<H>,
        peer: Peer,
        role: Role,
        unregistered: Option<mpsc::UnboundedSender<Command>>,
    ) -> (Self, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (out, frames) = mpsc::unbounded_channel();
        let connection = Self {
            hub,
            handler,
            peer,
            role,
            member: role == Role::Dialer,
            unregistered,
            out,
            offered: BTreeSet::new(),
            peer_offered: BTreeSet::new(),
            active: BTreeSet::new(),
            sessions: BTreeMap::new(),
        };
        (connection, frames)
    }

    async fn run(
        mut self,
        stream: TlsStream<TcpStream>,
        frames: mpsc::UnboundedReceiver<Vec<u8>>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        kept: Option<oneshot::Receiver<()>>,
    ) {
        let (mut reader, writer) = tokio::io::split(stream);
        tokio::spawn(write_frames(writer, frames));
        let (peer, addr, role) = (&self.peer.node, &self.peer.addr, self.role);
        debug!(%peer, %addr, ?role, "control connection open");
        let close = self.drive(&mut reader, &mut commands, kept).await;
        self.log_close(&close);
        if let Some(hub) = self.hub.upgrade() {
            hub.deregister(self.peer.id);
        }
    }

    async fn drive(
        &mut self,
        reader: &mut ReadHalf<TlsStream<TcpStream>>,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        kept: Option<oneshot::Receiver<()>>,
    ) -> Close {
        if self.role == Role::Dialer
            && let Err(close) = self.send_hello()
        {
            return close;
        }
        let mut frames = FrameReader::new();
        if !self.member {
            frames.limit_pending(MAX_UNADMITTED_PENDING);
        }
        let idle = sleep(if self.member {
            IDLE_TIMEOUT
        } else {
            HANDSHAKE_TIMEOUT
        });
        tokio::pin!(idle);
        let mut resync = interval_at(Instant::now() + RESYNC_INTERVAL, RESYNC_INTERVAL);
        resync.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let dropped = async move {
            match kept {
                Some(kept) => {
                    let _dropped = kept.await;
                }
                None => future::pending().await,
            }
        };
        tokio::pin!(dropped);
        loop {
            let step = tokio::select! {
                message = frames.next(reader) => match message {
                    Ok(message) => {
                        idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                        self.on_message(message)
                    }
                    Err(ChannelError::Closed) => Err(Close::PeerClosed),
                    Err(error) => Err(Close::Failed(error)),
                },
                command = commands.recv() => match command {
                    Some(command) => self.on_command(command),
                    None => Err(Close::Local),
                },
                _ = resync.tick(), if self.member && self.role == Role::Acceptor => {
                    self.resync()
                }
                () = &mut idle => Err(Close::TimedOut),
                () = &mut dropped => Err(Close::Local),
            };
            if let Err(close) = step {
                return close;
            }
            if self.member {
                frames.limit_pending(usize::MAX);
            }
        }
    }

    fn on_message(&mut self, message: Message) -> Result<(), Close> {
        match (self.member, message) {
            (false, Message::Hello { networks }) => self.on_first_hello(networks),
            (false, Message::JoinRequest { network, proof }) => self.on_join(network, proof),
            (false, _) => Err(Close::Violation("non-member sent neither hello nor join")),
            (true, Message::Hello { networks }) => self.on_hello(networks),
            (true, Message::Sync { network, bytes }) => self.on_sync(network, &bytes),
            (true, Message::Endpoints { network, entries }) => self.on_endpoints(network, entries),
            (
                true,
                Message::JoinRequest { .. }
                | Message::JoinAccepted { .. }
                | Message::JoinRejected { .. },
            ) => Err(Close::Violation("join message from a member")),
        }
    }

    fn on_command(&mut self, command: Command) -> Result<(), Close> {
        match command {
            Command::Changed(network) => {
                let hello_sent = self.refresh()?;
                if self.active.contains(&network) {
                    match self.role {
                        Role::Acceptor => self.start_sync(network)?,
                        Role::Dialer if !hello_sent => self.send_hello()?,
                        Role::Dialer => {}
                    }
                }
                Ok(())
            }
            Command::Endpoints(network, frame) => {
                if self.active.contains(&network) {
                    self.send_frame(frame.to_vec())?;
                }
                Ok(())
            }
        }
    }

    fn on_first_hello(&mut self, networks: Vec<NetworkId>) -> Result<(), Close> {
        self.offered = shared_networks(&*self.handler, self.peer.node);
        if self.offered.is_empty() {
            return Err(Close::NotMember);
        }
        self.register()?;
        self.send_hello()?;
        self.on_hello(networks)
    }

    fn on_hello(&mut self, networks: Vec<NetworkId>) -> Result<(), Close> {
        self.peer_offered = networks.into_iter().collect();
        self.update_active()?;
        match self.role {
            Role::Acceptor => self.sync_all_active(),
            Role::Dialer => Ok(()),
        }
    }

    fn on_join(&mut self, network: NetworkId, proof: Box<JoinProof>) -> Result<(), Close> {
        if proof.device != self.peer.node {
            return self.reject("the join proof names another device");
        }
        if !self.handler.is_member(network, self.peer.node) {
            if let Err(reason) = self.handler.handle_join(network, *proof) {
                return self.reject(&reason);
            }
            if !self.handler.is_member(network, self.peer.node) {
                return self.reject("the join was not admitted");
            }
            debug!(peer = %self.peer.node, %network, "admitted a device");
            self.handler.on_ledger_changed(network);
            self.broadcast_changed(network);
        }
        let genesis = self
            .handler
            .with_ledger(network, |ledger| {
                ledger.get(&ledger.genesis_id()).map(SignedOp::to_bytes)
            })
            .flatten()
            .ok_or(Close::Violation("joined a network without a ledger"))?;
        self.offered = shared_networks(&*self.handler, self.peer.node);
        self.peer_offered = BTreeSet::from([network]);
        self.register()?;
        self.send(&Message::JoinAccepted { genesis })?;
        self.update_active()?;
        self.start_sync(network)
    }

    fn reject(&self, reason: &str) -> Result<(), Close> {
        debug!(peer = %self.peer.node, reason, "rejected a join");
        self.send(&Message::rejection(reason))?;
        Err(Close::Rejected)
    }

    fn on_sync(&mut self, network: NetworkId, bytes: &[u8]) -> Result<(), Close> {
        if !self.active.contains(&network) {
            return Err(Close::Violation("sync for a network that is not shared"));
        }
        let slot = match self.sessions.entry(network) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) if self.role == Role::Dialer => {
                entry.insert(Slot::new(SyncSession::new(false, network)))
            }
            Entry::Vacant(_) => return Err(Close::Violation("sync message outside a session")),
        };
        let (result, changed, outgoing) = self
            .handler
            .with_ledger(network, |ledger| {
                let before = ledger.len();
                let result = slot.session.handle_message(ledger, bytes);
                let outgoing = drain(&mut slot.session, ledger);
                (result, ledger.len() != before, outgoing)
            })
            .ok_or(Close::Violation("no ledger for a shared network"))?;
        result.map_err(|error| Close::Failed(error.into()))?;
        slot.changed |= changed;
        let finished = slot.session.is_done().then_some((slot.rerun, slot.changed));
        for bytes in outgoing {
            self.send(&Message::Sync { network, bytes })?;
        }
        if changed {
            self.handler.on_ledger_changed(network);
        }
        if let Some((rerun, session_changed)) = finished {
            self.sessions.remove(&network);
            if session_changed {
                self.broadcast_changed(network);
                self.refresh()?;
            }
            if rerun {
                self.start_sync(network)?;
            }
        }
        Ok(())
    }

    fn on_endpoints(&self, network: NetworkId, entries: Entries) -> Result<(), Close> {
        if !self.active.contains(&network) {
            return Err(Close::Violation("gossip for a network that is not shared"));
        }
        self.handler.on_endpoints(network, self.peer.node, entries);
        Ok(())
    }

    fn refresh(&mut self) -> Result<bool, Close> {
        if !self.member {
            return Ok(false);
        }
        let shared = shared_networks(&*self.handler, self.peer.node);
        if shared.is_empty() {
            return Err(Close::NotMember);
        }
        if shared == self.offered {
            return Ok(false);
        }
        self.offered = shared;
        self.send_hello()?;
        let added = self.update_active()?;
        if self.role == Role::Acceptor {
            for network in added {
                self.start_sync(network)?;
            }
        }
        Ok(true)
    }

    fn update_active(&mut self) -> Result<Vec<NetworkId>, Close> {
        let active: BTreeSet<NetworkId> = self
            .offered
            .intersection(&self.peer_offered)
            .copied()
            .collect();
        let added: Vec<NetworkId> = active.difference(&self.active).copied().collect();
        self.sessions.retain(|network, _| active.contains(network));
        self.active = active;
        if let Some(hub) = self.hub.upgrade() {
            for network in &added {
                if let Some(frame) = hub.latest_endpoints(*network) {
                    self.send_frame(frame.to_vec())?;
                }
            }
        }
        Ok(added)
    }

    fn resync(&mut self) -> Result<(), Close> {
        let peer = self.peer.node;
        self.sessions.retain(|network, slot| {
            if slot.seen_by_resync {
                debug!(%peer, %network, "restarting a stalled sync session");
            }
            let running = !slot.seen_by_resync;
            slot.seen_by_resync = true;
            running
        });
        self.sync_all_active()
    }

    fn sync_all_active(&mut self) -> Result<(), Close> {
        let active: Vec<NetworkId> = self.active.iter().copied().collect();
        active
            .into_iter()
            .try_for_each(|network| self.start_sync(network))
    }

    fn start_sync(&mut self, network: NetworkId) -> Result<(), Close> {
        if !self.active.contains(&network) {
            return Ok(());
        }
        if let Some(slot) = self.sessions.get_mut(&network) {
            slot.rerun = true;
            return Ok(());
        }
        let mut session = SyncSession::new(true, network);
        let outgoing = self
            .handler
            .with_ledger(network, |ledger| drain(&mut session, ledger))
            .ok_or(Close::Violation("no ledger for a shared network"))?;
        self.sessions.insert(network, Slot::new(session));
        outgoing
            .into_iter()
            .try_for_each(|bytes| self.send(&Message::Sync { network, bytes }))
    }

    fn register(&mut self) -> Result<(), Close> {
        let commands = self
            .unregistered
            .take()
            .ok_or(Close::Violation("registered twice"))?;
        self.hub
            .upgrade()
            .ok_or(Close::Local)?
            .register(&self.peer, commands);
        self.member = true;
        Ok(())
    }

    fn broadcast_changed(&self, network: NetworkId) {
        if let Some(hub) = self.hub.upgrade() {
            hub.broadcast(&Command::Changed(network), Some(self.peer.id));
        }
    }

    fn send_hello(&self) -> Result<(), Close> {
        self.send(&Message::Hello {
            networks: self.offered.iter().copied().collect(),
        })
    }

    fn send(&self, message: &Message) -> Result<(), Close> {
        self.send_frame(message.to_frame().map_err(Close::Failed)?)
    }

    fn send_frame(&self, frame: Vec<u8>) -> Result<(), Close> {
        self.out.send(frame).map_err(|_| Close::PeerClosed)
    }

    fn log_close(&self, close: &Close) {
        let peer = &self.peer.node;
        match close {
            Close::Violation(reason) if self.member => {
                warn!(%peer, reason, "closing control connection: protocol violation");
            }
            Close::Failed(error) if self.member && !matches!(error, ChannelError::Io(_)) => {
                warn!(%peer, %error, "control connection failed");
            }
            Close::Violation(reason) => debug!(%peer, reason, "closing control connection"),
            Close::Failed(error) => debug!(%peer, %error, "control connection failed"),
            Close::NotMember => debug!(%peer, "closing control connection: not a member"),
            Close::TimedOut => debug!(%peer, "control connection timed out"),
            Close::PeerClosed | Close::Local | Close::Rejected => {
                debug!(%peer, "control connection closed");
            }
        }
    }
}

fn drain(session: &mut SyncSession, ledger: &Ledger) -> Vec<Vec<u8>> {
    std::iter::from_fn(|| session.poll_transmit(ledger)).collect()
}

async fn write_frames<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut frames: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(frame) = frames.recv().await {
        let written = timeout(WRITE_TIMEOUT, async {
            writer.write_all(&frame).await?;
            if frames.is_empty() {
                writer.flush().await?;
            }
            Ok::<(), std::io::Error>(())
        })
        .await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                debug!(%error, "control channel write failed");
                return;
            }
            Err(_) => {
                debug!("control channel write timed out");
                return;
            }
        }
    }
    if let Err(error) = writer.shutdown().await {
        debug!(%error, "control channel shutdown failed");
    }
}
