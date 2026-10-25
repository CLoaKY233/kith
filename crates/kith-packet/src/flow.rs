//! Five-tuple extraction for the packet filter and router.

use core::net::IpAddr;

use crate::{Error, IpPacket, field, ip_protocol};

/// The five-tuple identifying a flow.
///
/// Plain data: any combination of fields is a valid key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlowKey {
    /// Source address.
    pub src: IpAddr,
    /// Destination address.
    pub dst: IpAddr,
    /// IPv4 protocol or first IPv6 next header.
    pub protocol: u8,
    /// Source port, for TCP and UDP only.
    pub src_port: Option<u16>,
    /// Destination port, for TCP and UDP only.
    pub dst_port: Option<u16>,
}

impl FlowKey {
    /// Extracts the flow key from a raw IPv4 or IPv6 packet.
    ///
    /// Ports are read only from the first four transport bytes; the rest of
    /// the transport header is not validated. Ports are `None` for protocols
    /// other than TCP and UDP, for non-first IPv4 fragments, and for IPv6
    /// packets whose first next header is an extension header.
    ///
    /// Returns [`Error::Truncated`] for a TCP or UDP packet too short to carry ports.
    pub fn extract(packet: &[u8]) -> Result<Self, Error> {
        let ip = IpPacket::new_checked(packet)?;
        let protocol = ip.protocol();
        let carries_ports = matches!(protocol, ip_protocol::TCP | ip_protocol::UDP)
            && match &ip {
                IpPacket::V4(v4) => v4.fragment_offset() == 0,
                IpPacket::V6(_) => true,
            };
        let (src_port, dst_port) = if carries_ports {
            let payload = ip.payload();
            if payload.len() < 4 {
                return Err(Error::Truncated);
            }
            (
                Some(field::u16_at(payload, 0)),
                Some(field::u16_at(payload, 2)),
            )
        } else {
            (None, None)
        };
        Ok(Self {
            src: ip.src(),
            dst: ip.dst(),
            protocol,
            src_port,
            dst_port,
        })
    }
}
