use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::net::{TcpSocket, TcpStream, ToSocketAddrs, UdpSocket, lookup_host};

static MARK: AtomicU32 = AtomicU32::new(0);

/// Sets the `SO_MARK` every later kith socket carries, so exit-node routing skips them.
pub fn set_socket_mark(mark: Option<u32>) {
    MARK.store(mark.unwrap_or(0), Ordering::Relaxed);
}

pub(crate) fn socket_mark() -> Option<u32> {
    Some(MARK.load(Ordering::Relaxed)).filter(|mark| *mark != 0)
}

/// Connects a TCP stream that carries the process-wide socket mark.
pub async fn connect_tcp(addr: impl ToSocketAddrs) -> io::Result<TcpStream> {
    let mut last = None;
    for addr in lookup_host(addr).await? {
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        apply(&socket);
        match socket.connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address to connect to")))
}

/// Applies the process-wide socket mark to a UDP socket.
pub fn mark_udp(socket: &UdpSocket) {
    apply(socket);
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn apply<S: std::os::fd::AsFd>(socket: &S) {
    if let Some(mark) = socket_mark()
        && let Err(error) = socket2::SockRef::from(socket).set_mark(mark)
    {
        tracing::debug!(%error, mark, "cannot mark socket");
    }
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
const fn apply<S>(_socket: &S) {}
