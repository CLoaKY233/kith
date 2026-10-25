//! Wire framing: envelopes, versioning, length-prefixed codecs and TLV extensions.
//!
//! Every kith message on the wire, on disk or over IPC is an [`envelope`]:
//!
//! ```text
//! magic "kt" (2 bytes) | message type (u8) | version (varint) | postcard body
//! ```
//!
//! Decoding is strict, because signed bytes must have exactly one valid encoding. Bodies
//! must be deterministic: `BTreeMap` or sorted `Vec`s, never `HashMap`.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod envelope;
mod error;
mod ext;
pub mod frame;
pub mod registry;
mod varint;

pub use envelope::{Header, MAGIC, MAX_MESSAGE_LEN, MessageType};
pub use error::WireError;
pub use ext::{Extension, Extensions};
pub use frame::FrameDecoder;
