//! The UDP socket and its batched send and receive paths.

use std::io;
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};

use quinn_udp::{Transmit, UdpSocketState};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::Interest;
use tokio::net::UdpSocket;
use tracing::{debug, warn};

use crate::{RecvBatch, SendBatch, UdpError};

/// Socket buffer size requested in each direction; the OS may clamp it.
//
// Sized to absorb multi-gigabit bursts between event-loop turns.
pub(crate) const SOCKET_BUFFER_BYTES: usize = 7 * 1024 * 1024;

/// The smallest size worth trying when the OS rejects [`SOCKET_BUFFER_BYTES`] outright.
const MIN_SOCKET_BUFFER_BYTES: usize = 256 * 1024;

/// Socket buffer sizes as granted by the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSizes {
    /// Send buffer bytes.
    pub send: usize,
    /// Receive buffer bytes.
    pub recv: usize,
}

/// Socket configuration applied by [`UdpLink::bind_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindOptions {
    mark: Option<u32>,
}

impl BindOptions {
    /// Sets the Linux `SO_MARK`, so fwmark policy routing keeps the tunnel's own packets out of
    /// the tunnel. Elsewhere a mark fails with [`UdpError::Unsupported`].
    #[must_use]
    pub const fn mark(mut self, mark: Option<u32>) -> Self {
        self.mark = mark;
        self
    }
}

/// A bound UDP socket carrying WireGuard, disco and STUN.
#[derive(Debug)]
pub struct UdpLink {
    socket: UdpSocket,
    state: UdpSocketState,
    local_addr: SocketAddr,
    buffer_sizes: BufferSizes,
    /// Set once a segmented send failed because of segmentation; then it stays off.
    gso_disabled: AtomicBool,
}

impl UdpLink {
    /// Binds a socket to `addr` and registers it with the current tokio runtime.
    ///
    /// IPv4 gives an IPv4-only socket; `[::]` gives a dual-stack one on every OS.
    /// Must be called within a tokio runtime.
    pub fn bind(addr: SocketAddr) -> Result<Self, UdpError> {
        Self::bind_with(addr, BindOptions::default())
    }

    /// Like [`bind`](Self::bind), applying `options` before the socket is bound.
    pub fn bind_with(addr: SocketAddr, options: BindOptions) -> Result<Self, UdpError> {
        let bind_error = |source| UdpError::Bind { addr, source };
        let std_socket = open_socket(addr, options)?;
        let state = UdpSocketState::new((&std_socket).into()).map_err(bind_error)?;
        let buffer_sizes = grow_buffers(&state, &std_socket);
        std_socket.set_nonblocking(true).map_err(bind_error)?;
        let socket = UdpSocket::from_std(std_socket).map_err(bind_error)?;
        let local_addr = socket.local_addr().map_err(UdpError::LocalAddr)?;
        debug!(
            %local_addr,
            send_buffer = buffer_sizes.send,
            recv_buffer = buffer_sizes.recv,
            gso_segments = state.max_gso_segments(),
            gro_segments = state.gro_segments(),
            mark = ?options.mark,
            "UDP link bound",
        );
        Ok(Self {
            socket,
            state,
            local_addr,
            buffer_sizes,
            gso_disabled: AtomicBool::new(false),
        })
    }

    /// The address the socket is bound to, with the OS-assigned port if 0 was requested.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The socket buffer sizes the OS granted.
    pub fn buffer_sizes(&self) -> BufferSizes {
        self.buffer_sizes
    }

    /// How many datagrams one send may carry via segmentation offload. 1 means no offload.
    pub fn max_gso_segments(&self) -> usize {
        if self.gso_disabled.load(Ordering::Relaxed) {
            1
        } else {
            self.state.max_gso_segments()
        }
    }

    /// How many datagrams the kernel may coalesce into one receive slot. 1 means no offload.
    pub fn gro_segments(&self) -> usize {
        self.state.gro_segments()
    }

    /// Receives one syscall's worth of datagrams and returns how many arrived.
    pub async fn recv_batch(&self, batch: &mut RecvBatch) -> Result<usize, UdpError> {
        let (mut buffers, meta) = batch.buffers();
        let slots = self
            .socket
            .async_io(Interest::READABLE, || {
                self.state
                    .recv((&self.socket).into(), buffers.slots(), meta)
            })
            .await
            .map_err(UdpError::Recv)?;
        Ok(batch.set_filled(slots))
    }

    /// Sends every datagram in `batch` and returns the first failure.
    ///
    /// UDP is best effort, so one failing destination does not stop the rest.
    pub async fn send_batch(&self, batch: &SendBatch) -> Result<(), UdpError> {
        let mut first_error = None;
        for transmit in batch.transmits(self.max_gso_segments()) {
            if let Err(error) = self.send_transmit(transmit).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Some drivers reject GSO only at send time, so a failed segmented send is resent singly,
    /// and GSO is turned off if segmentation was at fault.
    async fn send_transmit(&self, transmit: Transmit<'_>) -> Result<(), UdpError> {
        let transmit = self.map_destination(transmit);
        let destination = transmit.destination;
        let send_error = move |source| UdpError::Send {
            destination,
            source,
        };
        let Err(segmented_error) = self.send_once(&transmit).await else {
            return Ok(());
        };
        let Some(segment_size) = transmit.segment_size else {
            return Err(send_error(segmented_error));
        };
        let mut first_error = None;
        let mut any_sent = false;
        for segment in transmit.contents.chunks(segment_size) {
            let single = Transmit {
                contents: segment,
                segment_size: None,
                ..transmit
            };
            match self.send_once(&single).await {
                Ok(()) => any_sent = true,
                Err(source) => {
                    first_error.get_or_insert(send_error(source));
                }
            }
        }
        // Single sends failing too blames the destination, unless quinn-udp already halted GSO.
        if any_sent || self.state.max_gso_segments() == 1 {
            self.disable_gso(&segmented_error);
        }
        first_error.map_or(Ok(()), Err)
    }

    fn disable_gso(&self, error: &io::Error) {
        if !self.gso_disabled.swap(true, Ordering::Relaxed) {
            warn!(
                local_addr = %self.local_addr,
                %error,
                "UDP segmentation offload failed; sending datagrams singly from now on",
            );
        }
    }

    async fn send_once(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        self.socket
            .async_io(Interest::WRITABLE, || {
                self.state.try_send((&self.socket).into(), transmit)
            })
            .await
    }

    /// An IPv6 socket can only address IPv4 peers by their IPv4-mapped form.
    fn map_destination<'a>(&self, mut transmit: Transmit<'a>) -> Transmit<'a> {
        if let (SocketAddr::V6(_), SocketAddr::V4(v4)) = (self.local_addr, transmit.destination) {
            transmit.destination = SocketAddr::new(v4.ip().to_ipv6_mapped().into(), v4.port());
        }
        transmit
    }
}

fn open_socket(addr: SocketAddr, options: BindOptions) -> Result<StdUdpSocket, UdpError> {
    let bind_error = |source| UdpError::Bind { addr, source };
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))
        .map_err(bind_error)?;
    if let SocketAddr::V6(v6) = addr
        && v6.ip().is_unspecified()
    {
        // Windows defaults to IPv6-only; Linux and macOS do not.
        socket.set_only_v6(false).map_err(bind_error)?;
    }
    if let Some(mark) = options.mark {
        set_mark(&socket, mark)?;
    } else if let Some(mark) = crate::mark::socket_mark()
        && let Err(error) = set_mark(&socket, mark)
    {
        tracing::debug!(%error, "cannot mark the UDP socket");
    }
    socket.bind(&addr.into()).map_err(bind_error)?;
    Ok(socket.into())
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn set_mark(socket: &Socket, mark: u32) -> Result<(), UdpError> {
    socket
        .set_mark(mark)
        .map_err(|source| UdpError::Mark { mark, source })
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn set_mark(_socket: &Socket, _mark: u32) -> Result<(), UdpError> {
    Err(UdpError::Unsupported { option: "SO_MARK" })
}

/// Requests [`SOCKET_BUFFER_BYTES`], halving on rejection; the OS default is an acceptable
/// fallback.
fn grow_buffers(state: &UdpSocketState, socket: &StdUdpSocket) -> BufferSizes {
    let mut bytes = SOCKET_BUFFER_BYTES;
    while bytes >= MIN_SOCKET_BUFFER_BYTES
        && state.set_send_buffer_size(socket.into(), bytes).is_err()
    {
        bytes /= 2;
    }
    let mut bytes = SOCKET_BUFFER_BYTES;
    while bytes >= MIN_SOCKET_BUFFER_BYTES
        && state.set_recv_buffer_size(socket.into(), bytes).is_err()
    {
        bytes /= 2;
    }
    BufferSizes {
        send: state.send_buffer_size(socket.into()).unwrap_or(0),
        recv: state.recv_buffer_size(socket.into()).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::Duration;

    use crate::{BindOptions, RecvBatch, SendBatch, UdpError, UdpLink};

    const DATAGRAMS: usize = 200;

    /// Sizes repeat in runs so GSO coalescing engages where available.
    fn payload(receiver: u8, index: usize) -> Vec<u8> {
        let len = [1200, 1200, 1200, 600, 1452, 1452, 37, 9][index % 8];
        let mut payload = vec![u8::try_from(index % 251).unwrap(); len];
        payload[0] = receiver;
        payload[1..3].copy_from_slice(&u16::try_from(index).unwrap().to_be_bytes());
        payload
    }

    /// Receives until `count` datagrams arrived, or until the link is idle for a second.
    async fn receive(link: &UdpLink, sender: SocketAddr, count: usize) -> BTreeMap<usize, Vec<u8>> {
        let mut batch = RecvBatch::compact(link);
        let mut received = BTreeMap::new();
        while received.len() < count
            && let Ok(result) =
                tokio::time::timeout(Duration::from_secs(1), link.recv_batch(&mut batch)).await
        {
            result.unwrap();
            for datagram in batch.datagrams() {
                assert_eq!(datagram.source, sender);
                let index = usize::from(u16::from_be_bytes([
                    datagram.payload[1],
                    datagram.payload[2],
                ]));
                assert!(received.insert(index, datagram.payload.to_vec()).is_none());
            }
        }
        received
    }

    #[tokio::test]
    async fn batches_deliver_every_payload_intact_from_the_right_source() {
        let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let sender = UdpLink::bind(loopback).unwrap();
        let receivers = [
            UdpLink::bind(loopback).unwrap(),
            UdpLink::bind(loopback).unwrap(),
        ];

        let mut expected: [BTreeMap<usize, Vec<u8>>; 2] = Default::default();
        let mut batch = SendBatch::new();
        for index in 0..DATAGRAMS {
            // Runs of five to one receiver, then the other, so runs both form and break.
            let receiver = (index / 5) % 2;
            let payload = payload(u8::try_from(receiver).unwrap(), index);
            batch
                .push(receivers[receiver].local_addr(), &payload)
                .unwrap();
            expected[receiver].insert(index, payload);
        }
        sender.send_batch(&batch).await.unwrap();

        for (receiver, expected) in receivers.iter().zip(expected) {
            let received = receive(receiver, sender.local_addr(), expected.len()).await;
            assert_eq!(received, expected);
        }
    }

    /// An unmarked socket in exit-node mode routes its own packets into the tunnel, so a
    /// requested mark must be applied or refused, never dropped.
    #[tokio::test]
    async fn requested_mark_is_applied_or_refused() {
        const MARK: u32 = 0x6b69;
        let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let result = UdpLink::bind_with(loopback, BindOptions::default().mark(Some(MARK)));

        #[cfg(any(target_os = "android", target_os = "linux"))]
        match result {
            Ok(link) => assert_eq!(socket2::SockRef::from(&link.socket).mark().unwrap(), MARK),
            Err(UdpError::Mark { mark, source }) => {
                assert_eq!(mark, MARK);
                assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
            }
            Err(error) => panic!("unexpected error: {error}"),
        }
        #[cfg(not(any(target_os = "android", target_os = "linux")))]
        assert!(matches!(result, Err(UdpError::Unsupported { .. })));
    }
}
