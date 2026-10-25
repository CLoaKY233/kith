//! The packet hot loop: TUN, filter, router, WireGuard and links.
//!
//! Plane: data. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.

#![forbid(unsafe_code)]
