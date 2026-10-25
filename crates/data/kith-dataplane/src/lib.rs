//! The packet hot loop: TUN, filter, router, WireGuard and links.
//!
//! Plane: data. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.
//!
//! One task ([`Dataplane::run`]) owns every core and all I/O, so the hot path takes no locks.
//! Each wakeup reads the clock once, handles one source (TUN, UDP, a command or a timer),
//! drives due WireGuard timers and flushes UDP once. Buffers are allocated up front.

#![forbid(unsafe_code)]

mod api;
mod engine;
mod packet;
mod tun_device;

#[cfg(test)]
mod tests;

pub use api::{
    Dataplane, DataplaneConfig, DataplaneError, DataplaneHandle, Endpoint, Event, PeerSpec,
};
