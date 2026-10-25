//! Transport and link abstractions plus the fallback ladder scheduler.
//!
//! Plane: connectivity. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
