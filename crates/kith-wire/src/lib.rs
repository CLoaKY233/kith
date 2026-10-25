//! Wire framing: envelopes, versioning, length-prefixed codecs and TLV extensions.
//!
//! Plane: foundation. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
