//! Operating-system integration: routes, DNS configuration, forwarding and NAT.
//!
//! The OS is driven through its standard tools (`ip`, `route`, `nft`, `pfctl`,
//! `resolvectl`, ...) invoked as argument vectors, never through a shell. Every
//! argument is validated before it is run, so injection is impossible by
//! construction. All side effects go through the [`System`] trait so tests can
//! substitute a fake.
//!
//! OS state must survive crashes, so there is no RAII guard: [`Sys::apply`] and
//! [`Sys::revert`] are idempotent, and `revert` restores from state files kept in
//! the state directory, so it also cleans up after a crashed run.
//!
//! This crate may use `unsafe`, but currently needs none.

mod command;
mod error;
mod linux;
mod macos;
mod plan;
mod sys;
mod validate;

#[cfg(test)]
mod tests;

pub use command::{Cmd, OsSystem, Output, System};
pub use error::Error;
pub use plan::{Dns, ExitNode, Nat, Plan};
pub use sys::{Platform, Sys};
pub use validate::{Domain, Interface};

/// Firewall mark the tunnel's own UDP socket must carry on Linux in exit-node mode.
pub const FWMARK: u32 = 0x6b69;

/// Linux policy-routing table holding the exit-node default route.
pub const ROUTE_TABLE: u32 = 0x6b69;

/// The `SO_MARK` value the UDP driver must set on its socket, if the platform needs one.
///
/// Setting it needs `setsockopt(SOL_SOCKET, SO_MARK)` (and `CAP_NET_ADMIN`), which
/// this crate leaves to the UDP driver.
pub const fn socket_mark(platform: Platform) -> Option<u32> {
    match platform {
        Platform::Linux => Some(FWMARK),
        Platform::MacOs => None,
    }
}
