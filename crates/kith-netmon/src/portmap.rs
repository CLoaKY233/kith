//! PCP-then-NAT-PMP port-mapping driver.

use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout_at};

use crate::codec::{
    self, NatPmpAddressResponse, NatPmpMapRequest, NatPmpMapResponse, PcpMapRequest, PcpMapResponse,
};

/// PCP initial retransmission time (RFC 6887 §8.1.1).
const PCP_INITIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// NAT-PMP initial retransmission time (RFC 6886 §3.1).
const NATPMP_INITIAL_TIMEOUT: Duration = Duration::from_millis(250);
/// How long to wait for the router to acknowledge a delete on shutdown.
const DELETE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long to wait before trying again after every protocol failed.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);

/// Settings for [`PortMapper::spawn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortMapConfig {
    /// The local UDP port to expose.
    pub internal_port: u16,
    /// The lifetime to request.
    pub lifetime: Duration,
    /// The PCP/NAT-PMP server; `None` discovers the default gateway on port 5351.
    pub gateway: Option<SocketAddr>,
    /// PCP transmissions before falling back; RFC 6887 retries forever, which would block NAT-PMP.
    pub pcp_attempts: u32,
    /// NAT-PMP transmissions per request (RFC 6886 §3.1 uses 9).
    pub natpmp_attempts: u32,
}

impl PortMapConfig {
    /// Defaults for `internal_port`: two hours, discovered gateway, 2 PCP and 9 NAT-PMP attempts.
    #[must_use]
    pub const fn new(internal_port: u16) -> Self {
        Self {
            internal_port,
            lifetime: Duration::from_secs(7200),
            gateway: None,
            pcp_attempts: 2,
            natpmp_attempts: 9,
        }
    }
}

/// A port mapping granted by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    /// The public address peers can reach.
    pub external: SocketAddr,
    /// How long the router keeps it without renewal.
    pub lifetime: Duration,
}

#[derive(Debug, thiserror::Error)]
enum PortMapError {
    #[error("no reply from the gateway")]
    Timeout,
    #[error("the gateway does not speak this protocol")]
    Unsupported,
    #[error("the gateway refused with result code {0}")]
    Refused(u16),
    #[error("the gateway granted a zero lifetime")]
    ZeroLifetime,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A background task that keeps one UDP port mapped, renewing at half the lifetime.
#[derive(Debug)]
pub struct PortMapper {
    mapping: watch::Receiver<Option<Mapping>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl PortMapper {
    /// Starts mapping; the current mapping is observable through [`PortMapper::mapping`].
    #[must_use]
    pub fn spawn(config: PortMapConfig) -> Self {
        let (publish, mapping) = watch::channel(None);
        let (shutdown, stop) = oneshot::channel();
        let task = tokio::spawn(run(config, publish, stop));
        Self {
            mapping,
            shutdown: Some(shutdown),
            task,
        }
    }

    /// The mapping currently held, if any.
    #[must_use]
    pub fn mapping(&self) -> Option<Mapping> {
        *self.mapping.borrow()
    }

    /// Waits until the mapping is gained, lost or changed, and returns the new value.
    pub async fn changed(&mut self) -> Option<Mapping> {
        self.mapping.changed().await.ok()?;
        *self.mapping.borrow_and_update()
    }

    /// Deletes the mapping at the router and stops the task.
    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        drop((&mut self.task).await);
    }
}

impl Drop for PortMapper {
    fn drop(&mut self) {
        if self.shutdown.is_some() {
            self.task.abort();
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Session {
    Pcp { nonce: [u8; 12], client: IpAddr },
    NatPmp,
}

async fn run(
    config: PortMapConfig,
    publish: watch::Sender<Option<Mapping>>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        let attempt = async {
            let gateway = match config.gateway {
                Some(gateway) => gateway,
                None => tokio::task::spawn_blocking(crate::gateway::default_gateway)
                    .await
                    .ok()
                    .flatten()
                    .map(|ip| SocketAddr::new(ip, codec::SERVER_PORT))
                    .ok_or(PortMapError::Timeout)?,
            };
            let unspecified = match gateway {
                SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                SocketAddr::V6(_) => IpAddr::V6(core::net::Ipv6Addr::UNSPECIFIED),
            };
            let socket = UdpSocket::bind(SocketAddr::new(unspecified, 0)).await?;
            socket.connect(gateway).await?;
            let (session, mapping) = acquire(&socket, &config).await?;
            Ok::<_, PortMapError>((socket, session, mapping))
        };
        let outcome = tokio::select! {
            outcome = attempt => outcome,
            _ = &mut stop => return,
        };
        let Ok((socket, session, mut mapping)) = outcome else {
            publish.send_replace(None);
            tokio::select! {
                () = sleep(RETRY_AFTER_FAILURE) => continue,
                _ = &mut stop => return,
            }
        };
        loop {
            publish.send_replace(Some(mapping));
            tokio::select! {
                () = sleep(mapping.lifetime / 2) => {}
                _ = &mut stop => {
                    delete(&socket, &config, session, mapping.external.port()).await;
                    return;
                }
            }
            let lifetime = lifetime_seconds(config.lifetime);
            let renewal = request(
                &socket,
                &config,
                session,
                lifetime,
                mapping.external.port(),
                attempts(&config, session),
            );
            let Ok(renewed) = renewal.await else {
                publish.send_replace(None);
                break;
            };
            mapping = renewed;
        }
    }
}

const fn attempts(config: &PortMapConfig, session: Session) -> u32 {
    match session {
        Session::Pcp { .. } => config.pcp_attempts,
        Session::NatPmp => config.natpmp_attempts,
    }
}

fn lifetime_seconds(lifetime: Duration) -> u32 {
    u32::try_from(lifetime.as_secs()).unwrap_or(u32::MAX).max(1)
}

async fn acquire(
    socket: &UdpSocket,
    config: &PortMapConfig,
) -> Result<(Session, Mapping), PortMapError> {
    let mut nonce = [0; 12];
    kith_crypto::fill_random(&mut nonce).map_err(|_| PortMapError::Timeout)?;
    let pcp = Session::Pcp {
        nonce,
        client: socket.local_addr()?.ip(),
    };
    let lifetime = lifetime_seconds(config.lifetime);
    match request(socket, config, pcp, lifetime, 0, config.pcp_attempts).await {
        Ok(mapping) => return Ok((pcp, mapping)),
        Err(PortMapError::Timeout | PortMapError::Unsupported) => {}
        Err(error) => return Err(error),
    }
    let mapping = request(
        socket,
        config,
        Session::NatPmp,
        lifetime,
        config.internal_port,
        config.natpmp_attempts,
    )
    .await?;
    Ok((Session::NatPmp, mapping))
}

/// Sends one best-effort delete (RFC 6886 §3.4, RFC 6887 §15) and briefly waits for the reply.
async fn delete(socket: &UdpSocket, config: &PortMapConfig, session: Session, external_port: u16) {
    let message = match session {
        Session::Pcp { nonce, client } => pcp_request(config, nonce, client, 0, external_port)
            .encode()
            .to_vec(),
        Session::NatPmp => NatPmpMapRequest {
            internal_port: config.internal_port,
            suggested_external_port: 0,
            lifetime: 0,
        }
        .encode()
        .to_vec(),
    };
    if socket.send(&message).await.is_ok() {
        let mut buffer = [0; 1100];
        drop(tokio::time::timeout(DELETE_TIMEOUT, socket.recv(&mut buffer)).await);
    }
}

fn pcp_request(
    config: &PortMapConfig,
    nonce: [u8; 12],
    client: IpAddr,
    lifetime: u32,
    suggested_external_port: u16,
) -> PcpMapRequest {
    PcpMapRequest {
        lifetime,
        client,
        nonce,
        internal_port: config.internal_port,
        suggested_external_port,
        suggested_external: match client {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(_) => IpAddr::V6(core::net::Ipv6Addr::UNSPECIFIED),
        },
    }
}

/// Sends a map request and returns the granted mapping.
async fn request(
    socket: &UdpSocket,
    config: &PortMapConfig,
    session: Session,
    lifetime: u32,
    suggested_external_port: u16,
    attempts: u32,
) -> Result<Mapping, PortMapError> {
    let internal_port = config.internal_port;
    match session {
        Session::Pcp { nonce, client } => {
            let message = pcp_request(config, nonce, client, lifetime, suggested_external_port);
            let response = exchange(
                socket,
                &message.encode(),
                attempts,
                PCP_INITIAL_TIMEOUT,
                |bytes| {
                    if codec::is_natpmp_unsupported_version(bytes) {
                        return Some(Err(PortMapError::Unsupported));
                    }
                    let response = PcpMapResponse::parse(bytes).ok()?;
                    (response.nonce == nonce && response.internal_port == internal_port)
                        .then_some(Ok(response))
                },
            )
            .await?;
            match response.result {
                0 => {}
                codec::PCP_UNSUPPORTED_VERSION => return Err(PortMapError::Unsupported),
                code => return Err(PortMapError::Refused(code.into())),
            }
            granted(
                SocketAddr::new(response.external, response.external_port),
                response.lifetime,
            )
        }
        Session::NatPmp => {
            let address = exchange(
                socket,
                &codec::encode_natpmp_address_request(),
                attempts,
                NATPMP_INITIAL_TIMEOUT,
                |bytes| NatPmpAddressResponse::parse(bytes).ok().map(Ok),
            )
            .await?;
            check_natpmp_result(address.result)?;
            let message = NatPmpMapRequest {
                internal_port,
                suggested_external_port,
                lifetime,
            };
            let response = exchange(
                socket,
                &message.encode(),
                attempts,
                NATPMP_INITIAL_TIMEOUT,
                |bytes| {
                    let response = NatPmpMapResponse::parse(bytes).ok()?;
                    (response.internal_port == internal_port).then_some(Ok(response))
                },
            )
            .await?;
            check_natpmp_result(response.result)?;
            granted(
                SocketAddr::new(IpAddr::V4(address.external), response.external_port),
                response.lifetime,
            )
        }
    }
}

fn check_natpmp_result(result: u16) -> Result<(), PortMapError> {
    match result {
        0 => Ok(()),
        codec::NATPMP_UNSUPPORTED_VERSION => Err(PortMapError::Unsupported),
        code => Err(PortMapError::Refused(code)),
    }
}

fn granted(external: SocketAddr, granted: u32) -> Result<Mapping, PortMapError> {
    if granted == 0 {
        return Err(PortMapError::ZeroLifetime);
    }
    Ok(Mapping {
        external,
        lifetime: Duration::from_secs(granted.into()),
    })
}

/// Transmits `message`, doubling the wait after each silent attempt, until `accept` matches a reply.
async fn exchange<T>(
    socket: &UdpSocket,
    message: &[u8],
    attempts: u32,
    initial: Duration,
    accept: impl Fn(&[u8]) -> Option<Result<T, PortMapError>>,
) -> Result<T, PortMapError> {
    let mut wait = initial;
    let mut buffer = [0; 1100];
    for _ in 0..attempts {
        socket.send(message).await?;
        let deadline = Instant::now() + wait;
        while let Ok(received) = timeout_at(deadline, socket.recv(&mut buffer)).await {
            // ICMP port-unreachable surfaces as a receive error on a connected socket; keep waiting.
            let Ok(length) = received else { continue };
            if let Some(outcome) = buffer.get(..length).and_then(&accept) {
                return outcome;
            }
        }
        wait = wait.saturating_mul(2);
    }
    Err(PortMapError::Timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTERNAL: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 9);
    const GRANTED: u32 = 120;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Seen {
        Pcp(u32),
        NatPmpAddress,
        NatPmpMap(u32),
    }

    async fn fake_router(
        speaks_pcp: bool,
    ) -> (
        SocketAddr,
        tokio::sync::mpsc::UnboundedReceiver<(Instant, Seen)>,
    ) {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let (log, seen) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut buffer = [0; 1100];
            loop {
                let (length, peer) = socket.recv_from(&mut buffer).await.unwrap();
                let bytes = &buffer[..length];
                let reply = if let Ok(request) = PcpMapRequest::parse(bytes) {
                    log.send((Instant::now(), Seen::Pcp(request.lifetime)))
                        .unwrap();
                    if speaks_pcp {
                        PcpMapResponse {
                            result: 0,
                            lifetime: request.lifetime.min(GRANTED),
                            epoch: 1,
                            nonce: request.nonce,
                            internal_port: request.internal_port,
                            external_port: 50000,
                            external: IpAddr::V4(EXTERNAL),
                        }
                        .encode()
                        .to_vec()
                    } else {
                        vec![0, 0x81, 0, 1, 0, 0, 0, 0]
                    }
                } else if codec::parse_natpmp_address_request(bytes).is_ok() {
                    log.send((Instant::now(), Seen::NatPmpAddress)).unwrap();
                    NatPmpAddressResponse {
                        result: 0,
                        epoch: 1,
                        external: EXTERNAL,
                    }
                    .encode()
                    .to_vec()
                } else {
                    let request = NatPmpMapRequest::parse(bytes).unwrap();
                    log.send((Instant::now(), Seen::NatPmpMap(request.lifetime)))
                        .unwrap();
                    NatPmpMapResponse {
                        result: 0,
                        epoch: 1,
                        internal_port: request.internal_port,
                        external_port: 50000,
                        lifetime: request.lifetime.min(GRANTED),
                    }
                    .encode()
                    .to_vec()
                };
                socket.send_to(&reply, peer).await.unwrap();
            }
        });
        (address, seen)
    }

    #[tokio::test(start_paused = true)]
    async fn maps_with_pcp_or_falls_back_to_natpmp_and_renews_at_half_lifetime() {
        for speaks_pcp in [true, false] {
            let (gateway, mut seen) = fake_router(speaks_pcp).await;
            let mut config = PortMapConfig::new(41641);
            config.gateway = Some(gateway);
            let mut mapper = PortMapper::spawn(config);

            let expected = Mapping {
                external: SocketAddr::new(IpAddr::V4(EXTERNAL), 50000),
                lifetime: Duration::from_secs(GRANTED.into()),
            };
            assert_eq!(mapper.changed().await, Some(expected));
            let mapped_at = Instant::now();
            let mut log = Vec::new();
            while let Ok((_, request)) = seen.try_recv() {
                // Paused time may auto-advance past a retransmit timer before loopback delivers.
                if log.last() != Some(&request) {
                    log.push(request);
                }
            }
            let first: &[Seen] = if speaks_pcp {
                &[Seen::Pcp(7200)]
            } else {
                &[Seen::Pcp(7200), Seen::NatPmpAddress, Seen::NatPmpMap(7200)]
            };
            assert_eq!(log, first);

            assert_eq!(mapper.changed().await, Some(expected));
            let renewed_after = Instant::now() - mapped_at;
            let half = Duration::from_secs(u64::from(GRANTED / 2));
            assert!(
                renewed_after >= half && renewed_after < half + PCP_INITIAL_TIMEOUT,
                "{renewed_after:?}"
            );

            mapper.shutdown().await;
            let expected_delete = if speaks_pcp {
                Seen::Pcp(0)
            } else {
                Seen::NatPmpMap(0)
            };
            while seen.recv().await.unwrap().1 != expected_delete {}
        }
    }
}
