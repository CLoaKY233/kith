//! Access-control policy model and compiler, plus deterministic address allocation.
//!
//! Plane: trust. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! This crate answers two questions every device must answer identically, from the same
//! replicated ledger, without asking anyone:
//!
//! 1. **Who may talk to whom?** A [`Policy`] is a default-deny list of allow [`Rule`]s.
//!    Each rule names source devices by [`Selector`] (everyone, one device, or a tag) and
//!    the [`Target`]s they may reach, with a [`Protocol`] and a [`PortSet`]. [`compile`]
//!    turns the policy into the [`CompiledFilter`] of one device: the inbound traffic it
//!    accepts, laid out for per-packet checks that never allocate.
//! 2. **Which mesh addresses does each device get?** [`allocate`] derives every member's
//!    IPv4 and IPv6 addresses from a hash of its [`NodeId`](kith_types::NodeId), so no
//!    server hands out leases and existing devices keep their addresses as others join.
//!
//! A brand-new network starts with [`Policy::default_allow_all`], so a personal network
//! just works: every device can reach every other one, and the owner tightens the policy
//! later. An empty policy ([`Policy::deny_all`]) allows nothing.
//!
//! The ledger stores the policy as opaque bytes. [`Policy::to_bytes`] and
//! [`Policy::from_bytes`] define that encoding: a `kith-wire` envelope of type
//! [`POLICY_MESSAGE_TYPE`], version [`POLICY_VERSION`], decoded strictly and re-validated.
//!
//! # Hash contexts
//!
//! - [`ALLOCATION_CONTEXT`]: `"kith 2026-10 address allocation v1"`.

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
