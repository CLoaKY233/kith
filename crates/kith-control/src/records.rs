//! Well-known ledger record keys.
//!
//! Unknown keys are ignored, so newer software can add keys without breaking older devices.
//!
//! - [`POLICY`]: a [`Policy::to_bytes`](kith_policy::Policy::to_bytes) encoding. Absent
//!   means [`Policy::default_allow_all`](kith_policy::Policy::default_allow_all).
//!   Undecodable means deny-all (except the device's own traffic) plus a warning: failing
//!   open would silently undo a policy meant to restrict traffic.
//! - [`ROUTE_PREFIX`]`<prefix>`, such as `route/192.168.1.0/24`: a subnet advertisement
//!   whose value is the advertiser's raw 32-byte [`NodeId`]. The prefix must be canonical;
//!   routes from non-members are dropped with a warning.
//! - [`DNS_PREFIX`]`<name>`: reserved for the naming plane.
//!
//! TODO: define the value format of `dns/<name>` records.

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
