//! The packet hot loop: TUN, filter, router, WireGuard and links.
//!
//! Plane: data. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.
//!
//! One task ([`Dataplane::run`]) owns every core and all I/O, so the hot path takes no locks.
//! Each wakeup reads the clock once, handles one source (TUN, UDP, a command or a timer),
//! then services only the peers it touched or whose deadline passed, and flushes UDP once.
//! Peer deadlines live in a min-heap, so a wakeup costs O(touched peers), not O(peers).
//! Buffers are allocated up front.

#![forbid(unsafe_code)]

mod api;
mod engine;
#[cfg(any(test, feature = "test-support"))]
mod memory_tun;
mod packet;
mod timers;
mod tun_device;

#[cfg(test)]
mod tests;

pub use api::{
    Dataplane, DataplaneConfig, DataplaneError, DataplaneHandle, DropCounters, Endpoint, Event,
    ExternalInterface, ExternalPeer, ExternalPeerStats, PeerSpec, PeerStats, Stats,
};
#[cfg(any(test, feature = "test-support"))]
pub use memory_tun::MemoryTun;
