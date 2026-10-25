//! Deterministic multi-node network simulator for testing the sans-IO cores.
//!
//! Plane: tooling. Library. See `docs/architecture.md`.
//!
//! [`Sim`] owns virtual time, a seeded [`Rng`] and the network; callers own their nodes and
//! pull [`Delivery`]s in time order. Everything derives from the seed, which is printed if
//! a test panics while the simulation is alive, so any failure can be replayed.

#![forbid(unsafe_code)]

mod nat;
mod net;
mod rng;

#[cfg(test)]
mod tests;

pub use nat::{Filtering, Mapping, NatConfig};
pub use net::{Delivery, HostId, LinkConfig, Sim};
pub use rng::Rng;
