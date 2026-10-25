//! UDP link driver with batched, offloaded I/O.
//!
//! Plane: connectivity. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.

#![forbid(unsafe_code)]
