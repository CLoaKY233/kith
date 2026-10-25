use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use kith_naming::udp_payload_size;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tracing::{debug, warn};

/// Most queries forwarded at once; further queries get SERVFAIL.
const MAX_IN_FLIGHT: usize = 1024;
/// How long one upstream gets before the next in the group is tried.
pub(crate) const DEFAULT_UPSTREAM_TIMEOUT: Duration = Duration::from_secs(2);

const HEADER_LEN: usize = 12;
const FLAG_RESPONSE: u8 = 0x80;
const FLAG_TRUNCATED: u8 = 0x02;
const MAX_MESSAGE: usize = 65535;
const OUTAGE_WARNING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct Pending {
    upstream: SocketAddr,
    reply: oneshot::Sender<Vec<u8>>,
}

type PendingTable = Mutex<HashMap<u16, Pending>>;

/// Where a query came from, which bounds the reply size the client can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientTransport {
    Udp,
    Tcp,
}

/// UDP forwarder that remaps transaction IDs and only accepts replies from the
/// upstream that was asked.
#[derive(Debug)]
pub(crate) struct Forwarder {
    sockets: [Option<Arc<UdpSocket>>; 2],
    pending: Arc<PendingTable>,
    slots: Arc<Semaphore>,
    upstream_timeout: Duration,
    readers: Vec<JoinHandle<()>>,
    last_outage_warning: Mutex<Option<Instant>>,
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
    }
}

impl Forwarder {
    pub(crate) async fn new(upstream_timeout: Duration) -> io::Result<Self> {
        let v4 = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        // IPv6 may be disabled on the host; IPv6 upstreams then fail over like dead ones.
        let v6 = UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).await.ok();
        let pending: Arc<PendingTable> = Arc::default();
        let sockets = [Some(Arc::new(v4)), v6.map(Arc::new)];
        let readers = sockets
            .iter()
            .flatten()
            .map(|socket| tokio::spawn(read_replies(Arc::clone(socket), Arc::clone(&pending))))
            .collect();
        Ok(Self {
            sockets,
            pending,
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            upstream_timeout,
            readers,
            last_outage_warning: Mutex::new(None),
        })
    }

    /// A slot in the in-flight table, or `None` when it is full.
    pub(crate) fn reserve(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.slots).try_acquire_owned().ok()
    }

    /// Tries each upstream in order and returns the reply carrying the client's ID.
    pub(crate) async fn forward(
        &self,
        query: &[u8],
        upstreams: &[SocketAddr],
        client: ClientTransport,
        _slot: &OwnedSemaphorePermit,
    ) -> Option<Vec<u8>> {
        let client_id = message_id(query)?;
        let mut remaining = upstreams.iter().peekable();
        while let Some(&upstream) = remaining.next() {
            if let Some(mut reply) = self.ask(query, upstream, client).await {
                reply
                    .get_mut(..2)?
                    .copy_from_slice(&client_id.to_be_bytes());
                return Some(reply);
            }
            if let Some(next) = remaining.peek() {
                debug!(%upstream, %next, "upstream did not answer; failing over");
            }
        }
        self.report_outage(upstreams);
        None
    }

    fn report_outage(&self, upstreams: &[SocketAddr]) {
        let now = Instant::now();
        let mut last = self
            .last_outage_warning
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.duration_since(at) < OUTAGE_WARNING_INTERVAL) {
            return;
        }
        *last = Some(now);
        drop(last);
        if upstreams.is_empty() {
            warn!("no upstream configured for a forwarded query; answering SERVFAIL");
        } else {
            warn!(?upstreams, "every upstream failed; answering SERVFAIL");
        }
    }

    async fn ask(
        &self,
        query: &[u8],
        upstream: SocketAddr,
        client: ClientTransport,
    ) -> Option<Vec<u8>> {
        let reply = self.ask_udp(query, upstream).await?;
        if !is_truncated(&reply) {
            return Some(reply);
        }
        match timeout(self.upstream_timeout, ask_tcp(query, upstream)).await {
            Ok(Some(full))
                if client == ClientTransport::Tcp
                    || full.len() <= usize::from(udp_payload_size(query)) =>
            {
                Some(full)
            }
            // The client can retry over TCP itself on the truncated reply.
            _ => Some(reply),
        }
    }

    async fn ask_udp(&self, query: &[u8], upstream: SocketAddr) -> Option<Vec<u8>> {
        let socket = self
            .sockets
            .get(usize::from(upstream.is_ipv6()))?
            .as_ref()?;
        let (id, receiver) = self.register(upstream)?;
        let mut outgoing = query.to_vec();
        outgoing.get_mut(..2)?.copy_from_slice(&id.to_be_bytes());
        let result = match socket.send_to(&outgoing, upstream).await {
            Ok(_) => timeout(self.upstream_timeout, receiver)
                .await
                .ok()
                .and_then(Result::ok),
            Err(_) => None,
        };
        self.table().remove(&id);
        result
    }

    fn register(&self, upstream: SocketAddr) -> Option<(u16, oneshot::Receiver<Vec<u8>>)> {
        let (reply, receiver) = oneshot::channel();
        let mut table = self.table();
        // The table never exceeds MAX_IN_FLIGHT of 65536 IDs, so a free one turns up fast.
        for _ in 0..64 {
            let id = random_id()?;
            if let std::collections::hash_map::Entry::Vacant(slot) = table.entry(id) {
                slot.insert(Pending { upstream, reply });
                return Some((id, receiver));
            }
        }
        None
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<u16, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn read_replies(socket: Arc<UdpSocket>, pending: Arc<PendingTable>) {
    let mut buffer = vec![0; MAX_MESSAGE];
    loop {
        let Ok((len, from)) = socket.recv_from(&mut buffer).await else {
            continue;
        };
        let Some(reply) = buffer.get(..len) else {
            continue;
        };
        if !is_response(reply) {
            continue;
        }
        let Some(id) = message_id(reply) else {
            continue;
        };
        let mut table = pending.lock().unwrap_or_else(PoisonError::into_inner);
        if table.get(&id).is_some_and(|entry| entry.upstream == from)
            && let Some(entry) = table.remove(&id)
        {
            drop(entry.reply.send(reply.to_vec()));
        }
    }
}

async fn ask_tcp(query: &[u8], upstream: SocketAddr) -> Option<Vec<u8>> {
    let id = random_id()?;
    let length = u16::try_from(query.len()).ok()?;
    let mut outgoing = Vec::with_capacity(query.len().checked_add(2)?);
    outgoing.extend_from_slice(&length.to_be_bytes());
    outgoing.extend_from_slice(query);
    outgoing.get_mut(2..4)?.copy_from_slice(&id.to_be_bytes());
    let mut stream = TcpStream::connect(upstream).await.ok()?;
    stream.write_all(&outgoing).await.ok()?;
    let reply_len = stream.read_u16().await.ok()?;
    let mut reply = vec![0; usize::from(reply_len)];
    stream.read_exact(&mut reply).await.ok()?;
    (is_response(&reply) && message_id(&reply) == Some(id)).then_some(reply)
}

fn random_id() -> Option<u16> {
    let mut bytes = [0; 2];
    getrandom::fill(&mut bytes).ok()?;
    Some(u16::from_be_bytes(bytes))
}

pub(crate) fn message_id(message: &[u8]) -> Option<u16> {
    let bytes = message.get(..2)?;
    Some(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]))
}

fn is_response(message: &[u8]) -> bool {
    message.len() >= HEADER_LEN
        && message
            .get(2)
            .is_some_and(|flags| flags & FLAG_RESPONSE != 0)
}

fn is_truncated(message: &[u8]) -> bool {
    message
        .get(2)
        .is_some_and(|flags| flags & FLAG_TRUNCATED != 0)
}

/// A SERVFAIL header for `query`, sent when no upstream answered.
pub(crate) fn server_failure(query: &[u8]) -> Option<Vec<u8>> {
    let header = query.get(..4)?;
    let recursion_desired = header.get(2)? & 0x01;
    Some(vec![
        *header.first()?,
        *header.get(1)?,
        FLAG_RESPONSE | recursion_desired,
        0x80 | 2,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
}
