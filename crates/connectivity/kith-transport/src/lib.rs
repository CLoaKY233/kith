//! Transport and link abstractions plus the fallback ladder scheduler.
//!
//! Sans-IO: no I/O, no clock, no runtime. Transports, cheapest to stealthiest:
//!
//! 1. plain WireGuard over UDP ([`TransportKind::Udp`]);
//! 2. obfuscated UDP, AmneziaWG-style ([`TransportKind::UdpObfuscated`]);
//! 3. QUIC on port 443 ([`TransportKind::Quic`]);
//! 4. TLS/WebSocket on port 443, CDN-compatible ([`TransportKind::WebSocket`]);
//! 5. REALITY-style TLS camouflage ([`TransportKind::Reality`], future).
//!
//! Drivers in the `kith-transport-*` crates perform the [`Ladder`]'s
//! [`Action`]s and report back with [`LinkEvent`]s.

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
