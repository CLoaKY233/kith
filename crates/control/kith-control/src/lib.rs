//! Per-network runtime that turns ledger state into a network map.
//!
//! Sans-IO: no I/O, no clock, no runtime. [`build_netmap`] is a pure function of a
//! [`NetworkView`](kith_ledger::NetworkView), so every device holding the same ops builds
//! the same [`NetMap`]: peers, local addresses, resolver, subnet routes, relays, custom
//! DNS records, exit nodes and the inbound filter. Bad records become [`Warning`]s and never
//! stop the build; only a mesh range too small for its members is an error.
//!
//! [`NetMap::diff`] reports minimal changes, because rebuilding WireGuard peers and filters
//! on every ledger change would drop sessions. [`Networks`] holds one map per joined
//! network and reports a [`Conflict`] when mesh ranges overlap, since the router keys
//! routes by destination address alone.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod names;
mod netmap;
mod networks;
pub mod records;

#[cfg(test)]
mod tests;

pub use netmap::{
    Client, ControlError, NetMap, NetMapDiff, Peer, Warning, addresses_for_newcomer, build_netmap,
};
pub use networks::{Conflict, Networks};
