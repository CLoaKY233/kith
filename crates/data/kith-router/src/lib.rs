//! Per-network longest-prefix-match routing from IP to peer.
//!
//! Sans-IO. All networks share one TUN, so routing happens here rather than in the OS.
//! Lookup never allocates.
//!
//! All networks share one [`PrefixTable`], because one TUN carries every network (see
//! `docs/architecture.md`). Sharing a table is only sound if no network can silently shadow
//! another, so [`Router::set_network`] enforces the rules below before it changes anything:
//! a rejected update leaves the router exactly as it was.
//!
//! # Conflict rules
//!
//! Each network has an IPv4 and an IPv6 [mesh range](MeshRanges), the block its peers'
//! addresses come from. Its routes are either **host routes** (`/32` or `/128`, a peer's own
//! address) or **subnet routes** (anything shorter, advertised by a peer).
//!
//! 1. **Mesh ranges are exclusive.** A network's mesh range must not overlap another
//!    network's mesh range of the same family in any way: equal, nested or partial.
//!    Violations are a [`ConflictError`] of kind [`ConflictKind::MeshRanges`].
//! 2. **Host routes stay home.** Every host route must lie inside its own network's mesh
//!    range, or [`SetNetworkError::HostOutsideMesh`].
//! 3. **Subnet routes keep out of other meshes.** A subnet route must not overlap another
//!    network's mesh range (inside it, or covering it), and a new mesh range must not overlap
//!    another network's subnet routes: [`ConflictKind::RouteInMeshRange`].
//! 4. **Subnet routes may nest.** Outside the mesh ranges, subnet routes of different
//!    networks may overlap, and longest-prefix match decides. Two networks holding the
//!    identical prefix is still a conflict: [`ConflictKind::SamePrefix`].
//!
//! The default routes `0.0.0.0/0` and `::/0` are never accepted as ordinary routes. They
//! belong to the exit-node mechanism: each network may name one exit peer
//! ([`Router::set_exit_peer`]) and at most one network is the active exit
//! ([`Router::set_active_exit`]). The exit is consulted only when no specific route in any
//! network matches.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

mod router;
mod table;

pub use router::{
    ConflictError, ConflictKind, ExitError, MeshRanges, Route, Router, SetNetworkError,
};
pub use table::{Iter, PrefixTable};
