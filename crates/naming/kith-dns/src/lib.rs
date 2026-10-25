//! Stub DNS server, upstream forwarding and split DNS.
//!
//! Plane: naming. Driver: owns I/O and wraps the sans-IO `kith-naming` engine. See
//! `docs/architecture.md`.
//!
//! Queries inside the zone are answered locally; the rest are forwarded unchanged to the
//! split-DNS group or the default upstreams, with a fresh random transaction ID per
//! attempt. No caching: upstream resolvers already cache, and correctness matters more.

#![forbid(unsafe_code)]

mod forward;
mod server;
mod state;
#[cfg(test)]
mod tests;

pub use server::DnsServer;
pub use state::DnsState;
