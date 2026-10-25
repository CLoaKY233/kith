//! Composition root that wires every plane together.
//!
//! Plane: management. Driver: owns I/O and wraps sans-IO cores. See `docs/node.md`.

#![forbid(unsafe_code)]

mod channel;
mod connectivity;
mod daemon;
mod dns;
mod external;
mod ipc;
mod network;
mod os;
mod server;
mod state;
mod types;

pub use daemon::{Node, NodeHandle, NodeOptions, TunMode, run};
pub use types::NodeError;
