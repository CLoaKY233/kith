//! Access-control policy model, parser and compiler, plus deterministic address allocation.
//!
//! Plane: trust. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
