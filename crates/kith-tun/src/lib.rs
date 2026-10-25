//! TUN device driver with batched reads and writes.
//!
//! Plane: data. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.

#![forbid(unsafe_code)]
