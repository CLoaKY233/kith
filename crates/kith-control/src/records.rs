//! Well-known ledger record keys.
//!
//! The ledger stores records as an opaque last-writer-wins map from string keys to bytes,
//! set by admins. These are the keys the control plane gives meaning to. Unknown keys are
//! ignored, so newer software can add keys without breaking older devices.
//!
//! - [`POLICY`]: the network's access policy, encoded by
//!   [`Policy::to_bytes`](kith_policy::Policy::to_bytes). When absent, the network uses
//!   [`Policy::default_allow_all`](kith_policy::Policy::default_allow_all), so a new
//!   personal network just works. When present but undecodable, the network falls back
//!   to deny-all (traffic from the device itself stays allowed) and reports a warning.
//!   Failing closed is the safe choice: an admin who wrote a policy meant to *restrict*
//!   traffic, and opening everything because the bytes are bad (or written by newer
//!   software) would silently undo that.
//! - [`ROUTE_PREFIX`]`<prefix>`, such as `route/192.168.1.0/24`: a subnet advertisement.
//!   The value is the 32 raw bytes of the advertising device's
//!   [`NodeId`]. The prefix must be canonical (no host bits set).
//!   Routes from non-members are dropped with a warning.
//! - [`DNS_PREFIX`]`<name>`: a custom DNS record for the naming plane.
//!
//! TODO: the value format of `dns/<name>` records is not defined yet; the naming plane
//! will define it, and this crate only reserves the key space.

use kith_types::{IpPrefix, NodeId};

/// The key holding the encoded network policy.
pub const POLICY: &str = "policy";

/// The prefix of subnet route advertisements: `route/<prefix>`.
pub const ROUTE_PREFIX: &str = "route/";

/// The prefix of custom DNS records: `dns/<name>`.
pub const DNS_PREFIX: &str = "dns/";

/// Parses a `route/<prefix>` record into the prefix and its advertising device.
///
/// Returns `None` if `key` is not a route key, and `Some(None)` if it is one but the key's
/// prefix or the value is malformed.
pub fn parse_route(key: &str, value: &[u8]) -> Option<Option<(IpPrefix, NodeId)>> {
    let prefix = key.strip_prefix(ROUTE_PREFIX)?;
    let parsed = prefix.parse::<IpPrefix>().ok();
    let device = <[u8; 32]>::try_from(value).ok().map(NodeId::from_bytes);
    Some(parsed.zip(device))
}
