//! The control channel between members: join, ledger sync and endpoint gossip.

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

/// The most nodes one endpoint gossip message describes.
pub const MAX_ENDPOINT_NODES: usize = 256;

/// The most addresses one endpoint gossip message lists per node.
pub const MAX_ENDPOINT_ADDRS: usize = 32;
