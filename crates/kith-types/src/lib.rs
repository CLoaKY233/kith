//! Shared identifiers, key newtypes, time and address-range types.
//!
//! Plane: foundation. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! Everything here is plain data: small, `Copy` where possible, totally ordered so that
//! every replica sorts the same way, and validated on construction *and* on
//! deserialization, so an invalid value can never exist in memory.

#![no_std]
#![forbid(unsafe_code)]

mod hex;
mod id;
mod net;
mod time;

pub use id::{NetworkId, NodeId, OpId, ParseIdError, ShortId};
pub use net::{IpPrefix, Ipv4Prefix, Ipv6Prefix, PrefixError};
pub use time::Timestamp;
