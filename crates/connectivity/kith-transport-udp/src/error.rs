//! Errors raised by the UDP link.

use std::io;
use std::net::SocketAddr;

/// Why a UDP link operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UdpError {
    /// The socket could not be bound or configured.
    #[error("cannot bind UDP socket to {addr}")]
    Bind {
        /// The address that was requested.
        addr: SocketAddr,
        /// The underlying OS error.
        #[source]
        source: io::Error,
    },
    /// The socket's local address could not be read.
    #[error("cannot read the UDP socket's local address")]
    LocalAddr(#[source] io::Error),
    /// Receiving failed.
    #[error("UDP receive failed")]
    Recv(#[source] io::Error),
    /// Sending to `destination` failed. Other datagrams in the batch were still attempted.
    #[error("UDP send to {destination} failed")]
    Send {
        /// Where the failed datagram was going.
        destination: SocketAddr,
        /// The underlying OS error.
        #[source]
        source: io::Error,
    },
    /// A datagram larger than [`MAX_DATAGRAM_LEN`](crate::MAX_DATAGRAM_LEN) was queued.
    #[error("datagram of {len} bytes exceeds the UDP maximum")]
    DatagramTooLarge {
        /// The rejected payload length.
        len: usize,
    },
}
