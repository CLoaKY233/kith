//! Routes other interfaces hold inside kith's address ranges, usually another VPN's.

use std::net::{IpAddr, Ipv4Addr};

use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix};

use crate::{Cmd, Error, Interface, Platform, Sys, System};

/// A route through another interface that overlaps one of kith's ranges.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteConflict {
    /// The other route's destination.
    pub prefix: IpPrefix,
    /// The interface it goes through.
    pub interface: String,
}

impl core::fmt::Display for RouteConflict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} via {}", self.prefix, self.interface)
    }
}

/// Parses one line of a routing table into its destination and interface.
type ParseRoute = fn(&str) -> Option<(IpPrefix, String)>;

/// Linux route types that describe local delivery, not another interface's claim.
const LOCAL_TYPES: [&str; 8] = [
    "local",
    "broadcast",
    "multicast",
    "unreachable",
    "prohibit",
    "blackhole",
    "throw",
    "anycast",
];

impl<S: System> Sys<S> {
    /// Routes through interfaces other than `own` and loopback that overlap `ranges`.
    pub fn route_conflicts(
        &mut self,
        ranges: &[IpPrefix],
        own: &Interface,
    ) -> Result<Vec<RouteConflict>, Error> {
        let mut routes = Vec::new();
        for v6 in [false, true] {
            let (cmd, parse): (Cmd, ParseRoute) = match self.platform {
                Platform::Linux => (
                    Cmd::new("ip")
                        .arg(if v6 { "-6" } else { "-4" })
                        .args(["route", "show", "table", "all"]),
                    linux_route,
                ),
                Platform::MacOs => (
                    Cmd::new("netstat")
                        .args(["-rn", "-f"])
                        .arg(if v6 { "inet6" } else { "inet" }),
                    macos_route,
                ),
            };
            routes.extend(self.exec(&cmd)?.stdout.lines().filter_map(parse));
        }
        let mut conflicts: Vec<RouteConflict> = routes
            .into_iter()
            .filter(|(_, dev)| dev != own.as_str() && !dev.starts_with("lo"))
            .filter(|(prefix, _)| ranges.iter().any(|range| overlaps(range, prefix)))
            .map(|(prefix, interface)| RouteConflict { prefix, interface })
            .collect();
        conflicts.sort();
        conflicts.dedup();
        Ok(conflicts)
    }
}

fn overlaps(a: &IpPrefix, b: &IpPrefix) -> bool {
    a.contains(first(b)) || b.contains(first(a))
}

fn first(prefix: &IpPrefix) -> IpAddr {
    match prefix {
        IpPrefix::V4(p) => p.addr().into(),
        IpPrefix::V6(p) => p.addr().into(),
    }
}

fn host(addr: IpAddr) -> Option<IpPrefix> {
    Some(match addr {
        IpAddr::V4(a) => Ipv4Prefix::new(a, 32).ok()?.into(),
        IpAddr::V6(a) => Ipv6Prefix::new(a, 128).ok()?.into(),
    })
}

/// `100.64.0.0/10 dev tailscale0 table 52 ...`
fn linux_route(line: &str) -> Option<(IpPrefix, String)> {
    let mut words = line.split_whitespace().peekable();
    if words.peek().is_some_and(|w| LOCAL_TYPES.contains(w)) {
        return None;
    }
    let dest = words.next()?;
    let prefix = dest
        .parse::<IpPrefix>()
        .ok()
        .or_else(|| host(dest.parse().ok()?))?;
    let dev = words.skip_while(|w| *w != "dev").nth(1)?;
    Some((prefix, dev.to_owned()))
}

/// `100.64/10  link#60  UCS  utun37` from `netstat -rn`, whose IPv4 networks drop zero octets.
fn macos_route(line: &str) -> Option<(IpPrefix, String)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (dest, dev) = (*words.first()?, *words.get(3)?);
    let dest = dest.split('%').next()?;
    let prefix = match dest.split_once('/') {
        Some((addr, len)) => {
            let len: u8 = len.parse().ok()?;
            match addr.parse::<IpAddr>() {
                Ok(IpAddr::V6(a)) => Ipv6Prefix::truncating(a, len).ok()?.into(),
                Ok(IpAddr::V4(a)) => Ipv4Prefix::truncating(a, len).ok()?.into(),
                Err(_) => Ipv4Prefix::truncating(short_v4(addr)?, len).ok()?.into(),
            }
        }
        None => host(dest.parse().ok()?)?,
    };
    Some((prefix, dev.to_owned()))
}

/// `100.64` is `100.64.0.0`.
fn short_v4(text: &str) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    for (slot, part) in octets.iter_mut().zip(text.split('.')) {
        *slot = part.parse().ok()?;
    }
    (text.split('.').count() <= 4).then(|| Ipv4Addr::from(octets))
}
