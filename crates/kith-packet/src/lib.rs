//! Zero-copy IPv4, IPv6, TCP, UDP and ICMP header views and checksums.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.

#![no_std]
#![forbid(unsafe_code)]
