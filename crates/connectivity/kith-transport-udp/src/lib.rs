//! UDP link driver with batched, offloaded I/O.
//!
//! Plane: connectivity. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.
//!
//! WireGuard, disco and STUN share one socket, and per-packet syscalls cap throughput, so I/O
//! is batched: [`SendBatch`] coalesces into GSO sends, [`RecvBatch`] splits GRO buffers, and
//! [`classify`] routes datagrams by header. Syscalls live in `quinn-udp`, keeping this crate
//! free of `unsafe`.
//!
//! | Platform | Batched receive | Send offload | Receive offload |
//! |----------|-----------------|--------------|-----------------|
//! | Linux, Android | `recvmmsg`, 32 per call | UDP GSO, up to 64 segments | UDP GRO |
//! | Windows | one per call | USO where the driver supports it | URO |
//! | macOS, iOS, BSDs | one per call | none | none |

#![forbid(unsafe_code)]

mod classify;
mod error;
mod link;
mod recv;
mod send;

pub use classify::{Kind, classify};
pub use error::UdpError;
pub use link::{BindOptions, BufferSizes, UdpLink};
pub use recv::{Datagram, MAX_SLOT_LEN, RecvBatch};
pub use send::SendBatch;
