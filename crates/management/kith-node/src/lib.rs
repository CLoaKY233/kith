//! Composition root that wires every plane together.
//!
//! Plane: management. Driver: owns I/O and wraps sans-IO cores. See `docs/node.md`.

#![forbid(unsafe_code)]

mod blocked;
mod channel;
mod connectivity;
mod daemon;
mod dns;
mod external;
mod ipc;
mod journal;
mod network;
mod os;
mod server;
mod state;
mod stealth;
mod types;

pub use daemon::{Node, NodeHandle, NodeOptions, TunMode, run};
pub use journal::Journal;
pub use types::NodeError;
