//! Shared identifiers, key newtypes, time and address-range types.
//!
//! Plain, totally ordered data, validated on construction *and* deserialization, so an
//! invalid value never exists in memory.

#![no_std]
#![forbid(unsafe_code)]

mod hex;
mod id;
mod net;
mod time;

pub use id::{NetworkId, NodeId, OpId, ParseIdError, ShortId};
pub use net::{IpPrefix, Ipv4Prefix, Ipv6Prefix, PrefixError};
pub use time::Timestamp;
