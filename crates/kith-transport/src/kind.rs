//! Transport kinds and sets of them.
//!
//! The ladder order is the declaration order of [`TransportKind`]: cheaper and
//! faster rungs first, stealthier and costlier rungs later. Every scheduling
//! decision in [`crate::Ladder`] is phrased in terms of that order, so it is
//! encoded once here, in the derived `Ord`.

use core::fmt;

/// One rung of the transport ladder.
///
/// Totally ordered by ladder position: `Udp < UdpObfuscated < Quic < WebSocket < Reality`.
/// A smaller kind is cheaper (less overhead, lower latency) and a larger kind
/// is harder to block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportKind {
    /// Plain WireGuard over UDP.
    Udp,
    /// WireGuard over UDP with an AmneziaWG-style packet transform.
    UdpObfuscated,
    /// WireGuard carried in QUIC datagrams on port 443.
    Quic,
    /// WireGuard carried in WebSocket messages over TLS on port 443. Works through CDNs.
    WebSocket,
    /// REALITY-style TLS camouflage. Not implemented by any driver yet, so it is off by default.
    Reality,
}

impl TransportKind {
    /// Number of kinds.
    pub const COUNT: usize = 5;

    /// Every kind in ladder order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Udp,
        Self::UdpObfuscated,
        Self::Quic,
        Self::WebSocket,
        Self::Reality,
    ];

    /// Bytes added by the carrier below WireGuard, with an IPv6 outer header
    /// (40 bytes) as the worst case.
    ///
    /// - `Udp`: IPv6 40 + UDP 8 = 48.
    /// - `UdpObfuscated`: 48 + 16. The AmneziaWG header remap is size-neutral;
    ///   the 16 bytes are an estimate for per-packet junk/padding.
    /// - `Quic`: 48 + 48. Estimate: short header (1 + up to 20 connection ID +
    ///   up to 4 packet number), AEAD tag 16, DATAGRAM frame type and length 3,
    ///   rounded up.
    /// - `WebSocket`: 104. Estimate: IPv6 40 + TCP 32 (with timestamps) + TLS 1.3
    ///   record 22 (header 5, content type 1, tag 16) + masked WebSocket
    ///   frame header up to 8 + 2-byte length prefix.
    /// - `Reality`: 96. Same as `WebSocket` without the WebSocket frame header.
    ///
    /// The stream-based kinds do not fragment on the path, but keeping
    /// packets under the path MTU avoids splitting one packet over two TCP segments.
    pub const fn carrier_overhead(self) -> u16 {
        match self {
            Self::Udp => 48,
            Self::UdpObfuscated => 64,
            Self::WebSocket => 104,
            Self::Quic | Self::Reality => 96,
        }
    }

    /// Position in [`Self::ALL`], from 0.
    pub const fn index(self) -> usize {
        match self {
            Self::Udp => 0,
            Self::UdpObfuscated => 1,
            Self::Quic => 2,
            Self::WebSocket => 3,
            Self::Reality => 4,
        }
    }

    const fn bit(self) -> u8 {
        1 << self.index()
    }

    /// Short lowercase name, stable for logs and config.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::UdpObfuscated => "udp-obfuscated",
            Self::Quic => "quic",
            Self::WebSocket => "websocket",
            Self::Reality => "reality",
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of enabled transport kinds.
///
/// The default is every kind except [`TransportKind::Reality`]. Users or
/// policy restrict it, for example to [`TransportSet::STEALTH`] on networks
/// where plain WireGuard must never be seen.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportSet(u8);

impl TransportSet {
    /// No kinds.
    pub const EMPTY: Self = Self(0);

    /// Every kind, including the experimental ones.
    pub const ALL: Self = Self(0b1_1111);

    /// Kinds that look like ordinary HTTPS traffic on port 443.
    pub const STEALTH: Self = Self::EMPTY
        .with(TransportKind::Quic)
        .with(TransportKind::WebSocket)
        .with(TransportKind::Reality);

    /// This set plus `kind`.
    #[must_use]
    pub const fn with(self, kind: TransportKind) -> Self {
        Self(self.0 | kind.bit())
    }

    /// This set minus `kind`.
    #[must_use]
    pub const fn without(self, kind: TransportKind) -> Self {
        Self(self.0 & !kind.bit())
    }

    /// Whether `kind` is in the set.
    pub const fn contains(self, kind: TransportKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Whether the set has no kinds.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Kinds both sets contain.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The cheapest kind in the set.
    pub fn cheapest(self) -> Option<TransportKind> {
        self.iter().next()
    }

    /// Kinds in the set, in ladder order.
    pub fn iter(self) -> impl DoubleEndedIterator<Item = TransportKind> {
        TransportKind::ALL
            .into_iter()
            .filter(move |kind| self.contains(*kind))
    }
}

impl Default for TransportSet {
    fn default() -> Self {
        Self::ALL.without(TransportKind::Reality)
    }
}

impl FromIterator<TransportKind> for TransportSet {
    fn from_iter<I: IntoIterator<Item = TransportKind>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl fmt::Debug for TransportSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ord_and_index_agree_with_ladder_order() {
        assert!(TransportKind::ALL.windows(2).all(|pair| pair[0] < pair[1]));
        for (index, kind) in TransportKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), index);
        }
    }

    #[test]
    fn default_set_excludes_reality() {
        let set = TransportSet::default();
        assert!(!set.contains(TransportKind::Reality));
        assert_eq!(set.iter().count(), 4);
        assert_eq!(set.cheapest(), Some(TransportKind::Udp));
    }
}
