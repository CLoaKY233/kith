//! Stateful packet filter that evaluates compiled policy rules.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
