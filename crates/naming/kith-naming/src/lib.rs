//! DNS zone built from the network map and custom records, and a sans-IO answer engine.
//!
//! Plane: naming. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # What it does
//!
//! - **Device names resolve automatically.** Each device becomes `<device>.<suffix>`
//!   with one A/AAAA record per mesh address, under *every* authoritative suffix, and
//!   each address gets a PTR record pointing at `<device>.<primary suffix>`.
//! - **Custom records.** Users add A, AAAA, CNAME, TXT and SRV records anywhere inside
//!   an authoritative suffix.
//! - **Own domains.** A built zone has one or more suffixes, such as the default
//!   `<network>.kith.internal` and a user's `home.example.com`.
//! - **Split DNS.** [`SplitDns`] sends chosen domains to chosen upstream groups; the
//!   longest matching suffix wins.
//!
//! The driver (`kith-dns`) owns sockets and upstream forwarding; this crate only turns
//! query bytes into a decision and, when authoritative, a response written straight into
//! the caller's buffer, with no allocation on the answer path.
//!
//! # Zone rules
//!
//! [`ZoneBuilder::build`] validates everything at once:
//!
//! - a CNAME is the only record at its name and never sits at an apex;
//! - custom records lie inside a suffix;
//! - a custom record at a device name fails the build unless the policy is
//!   [`OverridePolicy::CustomWins`], in which case the custom records replace all of
//!   that name's device records (the device's PTR records stay);
//! - every collection is bounded (see the `MAX_*` constants);
//! - the result is independent of insertion order.
//!
//! # Wildcards (RFC 4592, simplified)
//!
//! `*` may only be the whole leftmost label of an owner name. A query for a name that
//! does not exist is answered from `*.<closest encloser>`, where the closest encloser is
//! the longest existing ancestor; apexes and empty non-terminals exist, so
//! `*.home.example.com` does not cover `x.lab.home.example.com` when
//! `y.lab.home.example.com` exists. A wildcard never covers an existing name (a device,
//! say), and the synthesized records take the query name as their owner. Not supported:
//! wildcard CNAME/SRV *targets*, and DNSSEC wildcard proofs.
//!
//! # Answers
//!
//! - AA set; RA set because the driver forwards everything else.
//! - NXDOMAIN or NODATA carry a synthesized SOA in the authority section (RFC 2308).
//! - CNAME chains are followed within the zone up to `MAX_CNAME_CHAIN` links; an
//!   out-of-zone target is left to the client's resolver.
//! - The response fits the client's EDNS UDP size (512 without EDNS); otherwise only
//!   the header and question are sent, with TC set. The OPT record is echoed.
//! - Responses (QR set) and fragments shorter than a header are dropped. Bad queries get
//!   FORMERR, opcodes other than QUERY get NOTIMP, non-IN classes and zone transfers get
//!   REFUSED, and EDNS versions other than 0 get BADVERS.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

mod answer;
mod name;
mod record;
mod split;
mod wire;
mod zone;

pub use answer::{DnsError, Outcome, Rcode, answer, answer_stream, udp_payload_size};
pub use name::{DomainName, MAX_NAME_LEN, NameError};
pub use record::{MAX_TXT_STRING_LEN, Record, RecordData, RecordError, RecordType, Srv, Txt};
pub use split::{SplitDns, SplitDnsError, UpstreamGroupId};
pub use zone::{DEFAULT_DEVICE_TTL, OverridePolicy, Zone, ZoneBuilder, ZoneError};
