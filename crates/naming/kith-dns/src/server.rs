use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kith_naming::{Outcome, answer, answer_stream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Semaphore;

use crate::forward::{ClientTransport, DEFAULT_UPSTREAM_TIMEOUT, Forwarder, server_failure};
use crate::state::DnsState;

/// Largest query accepted from a client, over either transport.
const MAX_QUERY_SIZE: usize = 4096;
/// Most client TCP connections served at once; more are closed on accept.
const MAX_TCP_CONNECTIONS: usize = 128;
/// A client TCP connection with no complete query for this long is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

const MAX_MESSAGE: usize = 65535;

/// Stub DNS server listening on UDP and TCP at one address.
#[derive(Debug)]
pub struct DnsServer {
    udp: Arc<UdpSocket>,
    tcp: TcpListener,
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    state: Arc<DnsState>,
    forwarder: Forwarder,
}

impl DnsServer {
    /// Binds UDP and TCP on `listen` (TCP takes the UDP port when `listen` has port 0).
    pub async fn bind(listen: SocketAddr, state: Arc<DnsState>) -> io::Result<Self> {
        Self::bind_with_timeout(listen, state, DEFAULT_UPSTREAM_TIMEOUT).await
    }

    /// Like [`Self::bind`], giving each upstream `upstream_timeout` before the next.
    pub async fn bind_with_timeout(
        listen: SocketAddr,
        state: Arc<DnsState>,
        upstream_timeout: Duration,
    ) -> io::Result<Self> {
        let udp = UdpSocket::bind(listen).await?;
        let tcp = TcpListener::bind(udp.local_addr()?).await?;
        let forwarder = Forwarder::new(upstream_timeout).await?;
        Ok(Self {
            udp: Arc::new(udp),
            tcp,
            shared: Arc::new(Shared { state, forwarder }),
        })
    }

    /// The bound address, shared by UDP and TCP.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.udp.local_addr()
    }

    /// Serves queries until the future is dropped.
    pub async fn run(self) {
        let udp = tokio::spawn(serve_udp(Arc::clone(&self.udp), Arc::clone(&self.shared)));
        let _abort_udp = AbortOnDrop(udp);
        serve_tcp(self.tcp, self.shared).await;
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn serve_udp(socket: Arc<UdpSocket>, shared: Arc<Shared>) {
    // One spare byte detects oversized datagrams, which the OS truncates silently.
    let mut buffer = vec![0; MAX_QUERY_SIZE.saturating_add(1)];
    let mut out = vec![0; MAX_MESSAGE];
    loop {
        let Ok((len, client)) = socket.recv_from(&mut buffer).await else {
            continue;
        };
        let Some(query) = buffer.get(..len).filter(|_| len <= MAX_QUERY_SIZE) else {
            continue;
        };
        let snapshot = shared.state.snapshot();
        let upstreams = match answer(&snapshot.zone, &snapshot.split, query, &mut out) {
            Ok(Outcome::Answered(len) | Outcome::Rejected { len, .. }) => {
                if let Some(response) = out.get(..len) {
                    drop(socket.send_to(response, client).await);
                }
                continue;
            }
            Ok(Outcome::Forward { upstream_group }) => snapshot.group(upstream_group).to_vec(),
            Ok(Outcome::ForwardDefault) => snapshot.default_upstreams.clone(),
            Ok(_) | Err(_) => continue,
        };
        let query = query.to_vec();
        let Some(slot) = shared.forwarder.reserve() else {
            if let Some(failure) = server_failure(&query) {
                drop(socket.send_to(&failure, client).await);
            }
            continue;
        };
        let socket = Arc::clone(&socket);
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let reply = shared
                .forwarder
                .forward(&query, &upstreams, ClientTransport::Udp, &slot)
                .await
                .or_else(|| server_failure(&query));
            if let Some(reply) = reply {
                drop(socket.send_to(&reply, client).await);
            }
        });
    }
}

async fn serve_tcp(listener: TcpListener, shared: Arc<Shared>) {
    let connections = Arc::new(Semaphore::new(MAX_TCP_CONNECTIONS));
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            // Usually descriptor exhaustion; back off instead of spinning.
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            continue;
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            serve_connection(stream, &shared).await;
            drop(permit);
        });
    }
}

async fn serve_connection(mut stream: TcpStream, shared: &Shared) {
    let mut out = vec![0; MAX_MESSAGE];
    loop {
        let Ok(Some(query)) = tokio::time::timeout(IDLE_TIMEOUT, read_query(&mut stream)).await
        else {
            return;
        };
        let Some(response) = respond_stream(&query, shared, &mut out).await else {
            continue;
        };
        let Ok(length) = u16::try_from(response.len()) else {
            return;
        };
        let mut framed = Vec::with_capacity(response.len().saturating_add(2));
        framed.extend_from_slice(&length.to_be_bytes());
        framed.extend_from_slice(&response);
        if stream.write_all(&framed).await.is_err() {
            return;
        }
    }
}

async fn read_query(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let len = usize::from(stream.read_u16().await.ok()?);
    if len > MAX_QUERY_SIZE {
        return None;
    }
    let mut query = vec![0; len];
    stream.read_exact(&mut query).await.ok()?;
    Some(query)
}

async fn respond_stream(query: &[u8], shared: &Shared, out: &mut [u8]) -> Option<Vec<u8>> {
    let snapshot = shared.state.snapshot();
    let upstreams = match answer_stream(&snapshot.zone, &snapshot.split, query, out).ok()? {
        Outcome::Answered(len) | Outcome::Rejected { len, .. } => {
            return out.get(..len).map(<[u8]>::to_vec);
        }
        Outcome::Forward { upstream_group } => snapshot.group(upstream_group).to_vec(),
        Outcome::ForwardDefault => snapshot.default_upstreams.clone(),
        _ => return None,
    };
    let Some(slot) = shared.forwarder.reserve() else {
        return server_failure(query);
    };
    shared
        .forwarder
        .forward(query, &upstreams, ClientTransport::Tcp, &slot)
        .await
        .or_else(|| server_failure(query))
}
