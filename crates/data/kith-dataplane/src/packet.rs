//! The two IP header fields the dataplane reads.

use std::net::IpAddr;

/// The packet's source address.
pub(crate) fn source(packet: &[u8]) -> Option<IpAddr> {
    address(packet, 12, 8)
}

/// The packet's destination address.
pub(crate) fn destination(packet: &[u8]) -> Option<IpAddr> {
    address(packet, 16, 24)
}

fn address(packet: &[u8], v4_offset: usize, v6_offset: usize) -> Option<IpAddr> {
    match packet.first()? >> 4 {
        4 => {
            let bytes: [u8; 4] = packet.get(v4_offset..v4_offset + 4)?.try_into().ok()?;
            Some(IpAddr::from(bytes))
        }
        6 => {
            let bytes: [u8; 16] = packet.get(v6_offset..v6_offset + 16)?.try_into().ok()?;
            Some(IpAddr::from(bytes))
        }
        _ => None,
    }
}
