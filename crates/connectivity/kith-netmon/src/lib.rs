//! Network interface monitoring and port mapping.
//!
//! Plane: connectivity. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.
//!
//! Port mapping speaks PCP, then NAT-PMP; `UPnP` IGD is deliberately out of scope for now.

#![forbid(unsafe_code)]

mod codec;
mod gateway;
mod portmap;
mod snapshot;

pub use portmap::{Mapping, PortMapConfig, PortMapper};
pub use snapshot::{Change, InterfaceAddress, NetworkSnapshot, spawn_monitor};
