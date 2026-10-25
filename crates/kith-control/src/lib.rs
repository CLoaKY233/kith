//! Per-network runtime that turns ledger state into a network map.
//!
//! Plane: control. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
