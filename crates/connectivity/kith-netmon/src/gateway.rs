//! Default-gateway discovery without a shell.

use core::net::{IpAddr, Ipv4Addr};

/// Reads the IPv4 default gateway, or `None` when there is none or it cannot be read.
pub(crate) fn default_gateway() -> Option<IpAddr> {
    #[cfg(target_os = "linux")]
    {
        let table = std::fs::read_to_string("/proc/net/route").ok()?;
        parse_proc_net_route(&table).map(IpAddr::V4)
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/sbin/route")
            .args(["-n", "get", "default"])
            .output()
            .ok()?;
        parse_route_get(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg_attr(
    all(not(target_os = "linux"), not(test)),
    expect(dead_code, reason = "Linux only")
)]
fn parse_proc_net_route(table: &str) -> Option<Ipv4Addr> {
    table.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace().skip(1);
        let (destination, gateway) = (fields.next()?, fields.next()?);
        if destination != "00000000" {
            return None;
        }
        let raw = u32::from_str_radix(gateway, 16).ok()?;
        // The kernel prints the address in host byte order.
        let address = Ipv4Addr::from(raw.to_ne_bytes());
        (!address.is_unspecified()).then_some(address)
    })
}

#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "macOS only")
)]
fn parse_route_get(output: &str) -> Option<IpAddr> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("gateway:")?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_parsers_read_real_output() {
        let proc = "Iface\tDestination\tGateway \tFlags\n\
                    eth0\t0000A8C0\t00000000\t0001\n\
                    eth0\t00000000\t0101A8C0\t0003\n";
        if cfg!(target_endian = "little") {
            assert_eq!(
                parse_proc_net_route(proc),
                Some(Ipv4Addr::new(192, 168, 1, 1))
            );
        }
        let route =
            "   route to: default\ndestination: default\n    gateway: 10.0.0.1\n  interface: en0\n";
        assert_eq!(
            parse_route_get(route),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
    }
}
