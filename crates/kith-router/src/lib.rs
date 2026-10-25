//! Per-network longest-prefix-match routing from IP to peer.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! Every outbound packet is routed by destination address to a `(NetworkId, NodeId)`. A
//! device may be in several networks at once, all sharing one TUN, so routing happens here
//! rather than in the OS. The lookup runs per packet: it never allocates and never recurses.
//!
//! - [`PrefixTable`] is the longest-prefix-match structure: an arena-backed,
//!   path-compressed binary trie per address family.
//! - [`Router`] layers networks, conflict detection and exit-node fallback on top of one
//!   shared table. Its module docs state the exact conflict rule.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod router;
pub mod table;

pub use router::{
    ConflictError, ConflictKind, ExitError, MeshRanges, Route, Router, SetNetworkError,
};
pub use table::{Iter, PrefixTable};
