//! Transport and link abstractions plus the fallback ladder scheduler.
//!
//! Plane: connectivity. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! kith promises to work on any network. Networks that block WireGuard do so
//! in different ways, so a peer can be reached over several transports,
//! ordered from cheapest to stealthiest:
//!
//! 1. plain WireGuard over UDP ([`TransportKind::Udp`]);
//! 2. obfuscated UDP, AmneziaWG-style ([`TransportKind::UdpObfuscated`]);
//! 3. QUIC on port 443 ([`TransportKind::Quic`]);
//! 4. TLS/WebSocket on port 443, CDN-compatible ([`TransportKind::WebSocket`]);
//! 5. REALITY-style TLS camouflage ([`TransportKind::Reality`], future).
//!
//! This crate holds the shared vocabulary ([`TransportKind`], [`TransportSet`],
//! [`LinkId`], [`LinkEvent`], [`Mtu`], [`NetworkFingerprint`]) and the
//! [`Ladder`], a deterministic state machine that decides which rungs to try,
//! when, and which link to keep. Drivers in the `kith-transport-*` crates
//! perform the [`Action`]s it emits and report back with [`LinkEvent`]s.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

mod fingerprint;
mod kind;
mod ladder;
mod link;
mod mtu;

pub use fingerprint::NetworkFingerprint;
pub use kind::{TransportKind, TransportSet};
pub use ladder::{
    Action, BACKOFF_BASE, BACKOFF_CAP, CACHE_CAPACITY, CACHE_TTL, ConfigError, Ladder,
    LadderConfig, MAX_CACHE_CAPACITY, RungState, STAGGER, SUPERSEDE_GRACE, UPGRADE_PROBE_INTERVAL,
};
pub use link::{AttemptId, FailureReason, LinkEvent, LinkId};
pub use mtu::{Mtu, MtuError};
