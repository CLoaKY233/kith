//! TUN device driver with batched reads and writes.
//!
//! Linux uses GSO/GRO offload when available. macOS uses `utun`. On Windows
//! [`Tun::create`] returns [`TunError::Unsupported`] until kith ships `wintun.dll`.
//! Creating a device needs root, or `CAP_NET_ADMIN` on Linux.
//!
//! One device carries every joined network, so [`Tun::add_address`] and
//! [`Tun::remove_address`] change its addresses while it runs.

#![forbid(unsafe_code)]

mod address;
mod batch;
mod config;
mod device;

pub use batch::{BATCH_SIZE, BatchError, MAX_PACKET_CAPACITY, PacketBatch};
pub use config::{ConfigError, MAX_MTU, TunConfig};
pub use device::{Tun, TunError};
