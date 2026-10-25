//! Wire framing: envelopes, versioning, length-prefixed codecs and TLV extensions.
//!
//! Plane: foundation. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! Every kith message on the wire, on disk or over IPC is an [`envelope`]:
//!
//! ```text
//! magic "kt" (2 bytes) | message type (u8) | version (varint) | postcard body
//! ```
//!
//! Decoding is strict, because signed bytes must have exactly one valid encoding:
//! overlong varints, trailing bytes and oversized inputs are all errors.
//!
//! Bodies must be deterministic: use `BTreeMap` or sorted `Vec`s, never `HashMap`.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod envelope;
mod error;
pub mod ext;
pub mod frame;
pub mod registry;
pub mod varint;

pub use envelope::{Header, MAGIC, MAX_MESSAGE_LEN, MessageType};
pub use error::WireError;
pub use ext::{Extension, Extensions};
pub use frame::FrameDecoder;
