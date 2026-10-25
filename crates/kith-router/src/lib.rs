//! Per-network longest-prefix-match routing from IP to peer.
//!
//! Sans-IO. All networks share one TUN, so routing happens here rather than in the OS.
//! Lookup never allocates. [`Router`]'s module docs state the conflict rules.

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
