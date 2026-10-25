//! Access-control policy model and compiler, plus deterministic address allocation.
//!
//! Sans-IO core: no I/O, no clock, no runtime.
//!
//! - A [`Policy`] is a default-deny set of allow [`Rule`]s. [`compile`] turns it into the
//!   [`CompiledFilter`] of one device, which checks inbound packets without allocating.
//! - [`allocate`] derives each member's addresses from a hash of its
//!   [`NodeId`](kith_types::NodeId) under [`ALLOCATION_CONTEXT`]. Appending members never
//!   moves existing ones.
//!
//! New networks start with [`Policy::default_allow_all`]; [`Policy::deny_all`] allows
//! nothing. The ledger stores [`Policy::to_bytes`] opaquely as a [`POLICY_MESSAGE_TYPE`]
//! envelope, version [`POLICY_VERSION`].

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod allocate;
mod compile;
mod policy;
mod ports;
mod tag;

pub use allocate::{
    ALLOCATION_CONTEXT, Addresses, AllocationError, Family, allocate, resolver_address,
};
pub use compile::{CompiledFilter, FilterEntry, Member, Members, compile};
pub use policy::{
    MAX_RULES, MAX_SELECTORS_PER_RULE, POLICY_MESSAGE_TYPE, POLICY_VERSION, PacketProtocol, Policy,
    PolicyError, Protocol, Rule, Selector, Target,
};
pub use ports::{MAX_PORT_RANGES, PortRange, PortRanges, PortSet};
pub use tag::{MAX_TAG_LEN, TagError, TagName};
