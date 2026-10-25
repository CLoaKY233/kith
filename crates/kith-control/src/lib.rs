//! Per-network runtime that turns ledger state into a network map.
//!
//! Plane: control. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # What it does
//!
//! Every device derives its whole view of a network locally, from the replicated ledger.
//! [`build_netmap`] turns one [`NetworkView`](kith_ledger::NetworkView) into a [`NetMap`]:
//! the peers with their names, tags and mesh addresses, the local device's addresses, the
//! mesh resolver, subnet routes, and the compiled inbound packet filter. Drivers, the data
//! plane and the naming plane consume the map; none of them reads the ledger.
//!
//! The build is a pure function of the view, so every device holding the same ops builds
//! the same map, whatever order the ops arrived in. It never fails because of what an
//! admin wrote into a record: bad records become [`Warning`]s, and the rest of the map is
//! still built. Only a broken genesis (a mesh range too small to allocate from) is an
//! error.
//!
//! # Why diffs
//!
//! Rebuilding WireGuard peers and filters on every ledger change would drop sessions.
//! [`NetMap::diff`] reports exactly which peers came, went or changed, and whether the
//! filter, local addresses or routes moved, so consumers apply minimal updates.
//!
//! # Several networks
//!
//! A device can join many networks. [`Networks`] keeps one map per network and reports a
//! [`Conflict`] when two joined networks' mesh ranges overlap: the router keys routes by
//! destination address alone, so it cannot serve both, and the user should hear about it
//! before traffic goes to the wrong place.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod netmap;
mod networks;
pub mod records;

#[cfg(test)]
mod tests;

pub use netmap::{ControlError, NetMap, NetMapDiff, Peer, Warning, build_netmap};
pub use networks::{Conflict, Networks};
