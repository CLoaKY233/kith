//! Path and tunnel MTU arithmetic.

use crate::TransportKind;

/// A maximum transmission unit in bytes, at least [`Mtu::MIN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mtu(u16);

/// An MTU value below [`Mtu::MIN`], or a carrier whose overhead leaves less than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("MTU {0} is below the minimum of {min}", min = Mtu::MIN.get())]
pub struct MtuError(pub u16);

impl Mtu {
    /// Smallest MTU every IPv4 host must accept (RFC 791).
    pub const MIN: Self = Self(576);

    /// Typical Ethernet path MTU.
    pub const ETHERNET: Self = Self(1500);

    /// Bytes WireGuard adds to each transport data packet: a 16-byte header
    /// (type, receiver index, counter) plus a 16-byte Poly1305 tag.
    ///
    /// WireGuard also pads the plaintext to a multiple of 16 bytes, but never
    /// beyond the tunnel MTU, so padding does not reduce it.
    pub const WIREGUARD_OVERHEAD: u16 = 32;

    /// Validates `bytes`.
    pub const fn new(bytes: u16) -> Result<Self, MtuError> {
        if bytes < Self::MIN.0 {
            Err(MtuError(bytes))
        } else {
            Ok(Self(bytes))
        }
    }

    /// The value in bytes.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The inner (tunnel interface) MTU when this is the path MTU and packets
    /// travel over `kind`: path minus [`TransportKind::carrier_overhead`] minus
    /// [`Self::WIREGUARD_OVERHEAD`].
    pub fn tunnel(self, kind: TransportKind) -> Result<Self, MtuError> {
        let inner = self
            .0
            .saturating_sub(kind.carrier_overhead())
            .saturating_sub(Self::WIREGUARD_OVERHEAD);
        Self::new(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_udp_on_ethernet_is_the_classic_1420() {
        assert_eq!(
            Mtu::ETHERNET.tunnel(TransportKind::Udp).map(Mtu::get),
            Ok(1420)
        );
    }

    #[test]
    fn rejects_mtu_and_tunnel_below_minimum() {
        assert_eq!(Mtu::new(575), Err(MtuError(575)));
        assert_eq!(
            Mtu::new(600).and_then(|mtu| mtu.tunnel(TransportKind::Udp)),
            Err(MtuError(520))
        );
    }
}
