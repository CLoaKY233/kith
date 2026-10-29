//! The control channel between members: join, ledger sync, and gossip of endpoints and
//! of whether gateways' plain WireGuard members are online.

#![forbid(unsafe_code)]

mod channel;
mod connection;
mod error;
mod handler;
mod message;

#[cfg(test)]
mod tests;

pub use channel::{Channel, ChannelClient, ChannelServer};
pub use error::ChannelError;
pub use handler::ChannelHandler;
pub use message::Presence;

/// The most nodes one endpoint gossip message describes.
const MAX_ENDPOINT_NODES: usize = 256;

/// The most addresses one endpoint gossip message lists per node.
const MAX_ENDPOINT_ADDRS: usize = 32;
