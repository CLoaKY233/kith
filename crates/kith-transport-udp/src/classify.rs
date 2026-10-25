//! Demultiplexing of the protocols that share the UDP socket.
//!
//! Checks read only a fixed header and the length. Each rule also pins a length or magic so
//! one protocol cannot match another's rule.
//!
//! Obfuscated WireGuard (`kith-obfs`) looks like noise; it is classified after deobfuscation
//! upstream, so here it is [`Kind::Unknown`].

use kith_wire::registry::DISCO_MESSAGE;

/// Which protocol a datagram belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A WireGuard handshake, cookie reply or transport-data message.
    WireGuard,
    /// A STUN message (RFC 8489).
    Stun,
    /// A kith disco envelope.
    Disco,
    /// Anything else, including obfuscated WireGuard before deobfuscation.
    Unknown,
}

/// WireGuard message type bytes, each followed by three reserved zero bytes.
const WIREGUARD_INITIATION: u8 = 1;
const WIREGUARD_RESPONSE: u8 = 2;
const WIREGUARD_COOKIE_REPLY: u8 = 3;
const WIREGUARD_TRANSPORT: u8 = 4;

/// Fixed WireGuard message lengths from the protocol whitepaper.
const WIREGUARD_INITIATION_LEN: usize = 148;
const WIREGUARD_RESPONSE_LEN: usize = 92;
const WIREGUARD_COOKIE_REPLY_LEN: usize = 64;
/// Transport data: 16-byte header, plaintext padded to 16, 16-byte tag.
const WIREGUARD_TRANSPORT_MIN_LEN: usize = 32;
const WIREGUARD_TRANSPORT_ALIGN: usize = 16;

/// STUN header length and the magic cookie at bytes 4..8 (RFC 8489 §5).
const STUN_HEADER_LEN: usize = 20;
const STUN_MAGIC_COOKIE: [u8; 4] = 0x2112_A442_u32.to_be_bytes();

/// Classifies a datagram by its header and length.
pub fn classify(datagram: &[u8]) -> Kind {
    if is_wireguard(datagram) {
        Kind::WireGuard
    } else if is_stun(datagram) {
        Kind::Stun
    } else if is_disco(datagram) {
        Kind::Disco
    } else {
        Kind::Unknown
    }
}

fn is_wireguard(datagram: &[u8]) -> bool {
    let Some(&[message_type, 0, 0, 0]) = datagram.first_chunk::<4>() else {
        return false;
    };
    let len = datagram.len();
    match message_type {
        WIREGUARD_INITIATION => len == WIREGUARD_INITIATION_LEN,
        WIREGUARD_RESPONSE => len == WIREGUARD_RESPONSE_LEN,
        WIREGUARD_COOKIE_REPLY => len == WIREGUARD_COOKIE_REPLY_LEN,
        WIREGUARD_TRANSPORT => {
            len >= WIREGUARD_TRANSPORT_MIN_LEN && len.is_multiple_of(WIREGUARD_TRANSPORT_ALIGN)
        }
        _ => false,
    }
}

fn is_stun(datagram: &[u8]) -> bool {
    let Some(&[type_high, _, length_high, length_low, c0, c1, c2, c3]) =
        datagram.first_chunk::<8>()
    else {
        return false;
    };
    let body_len = usize::from(u16::from_be_bytes([length_high, length_low]));
    type_high & 0b1100_0000 == 0
        && [c0, c1, c2, c3] == STUN_MAGIC_COOKIE
        && body_len.is_multiple_of(4)
        && datagram.len() == STUN_HEADER_LEN + body_len
}

fn is_disco(datagram: &[u8]) -> bool {
    let Some(&[m0, m1, message_type]) = datagram.first_chunk::<3>() else {
        return false;
    };
    [m0, m1] == kith_wire::MAGIC && message_type == DISCO_MESSAGE.0
}

#[cfg(test)]
mod tests {
    use super::{Kind, classify};
    use kith_wire::registry::{DISCO_MESSAGE, LEDGER_SIGNED_OP};

    fn wireguard(message_type: u8, len: usize) -> Vec<u8> {
        let mut message = vec![0xa5; len];
        message[..4].copy_from_slice(&[message_type, 0, 0, 0]);
        message
    }

    /// The binding request header from the RFC 5769 §2.1 test vector, with its 88-byte body.
    fn stun_binding_request() -> Vec<u8> {
        let mut message = vec![
            0x00, 0x01, 0x00, 0x58, 0x21, 0x12, 0xa4, 0x42, 0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34,
            0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
        ];
        message.resize(20 + 0x58, 0);
        message
    }

    #[test]
    fn classify_distinguishes_every_protocol_and_rejects_lookalikes() {
        let disco = kith_wire::envelope::encode(DISCO_MESSAGE, 1, &[7_u8; 40][..]).unwrap();
        let ledger = kith_wire::envelope::encode(LEDGER_SIGNED_OP, 1, &[7_u8; 40][..]).unwrap();
        let mut stun_success = stun_binding_request();
        stun_success[..2].copy_from_slice(&[0x01, 0x01]);
        let mut stun_bad_length = stun_binding_request();
        stun_bad_length.push(0);
        let mut channel_data = stun_binding_request();
        channel_data[0] = 0x40;

        let cases: &[(&str, Vec<u8>, Kind)] = &[
            ("wg initiation", wireguard(1, 148), Kind::WireGuard),
            ("wg response", wireguard(2, 92), Kind::WireGuard),
            ("wg cookie reply", wireguard(3, 64), Kind::WireGuard),
            ("wg keepalive", wireguard(4, 32), Kind::WireGuard),
            ("wg data", wireguard(4, 32 + 1424), Kind::WireGuard),
            (
                "wg initiation, wrong length",
                wireguard(1, 149),
                Kind::Unknown,
            ),
            ("wg data, unpadded", wireguard(4, 33), Kind::Unknown),
            ("wg type 5", wireguard(5, 148), Kind::Unknown),
            (
                "wg nonzero reserved",
                vec![1, 0, 1, 0, 0, 0, 0, 0],
                Kind::Unknown,
            ),
            ("stun binding request", stun_binding_request(), Kind::Stun),
            ("stun binding success", stun_success, Kind::Stun),
            ("stun, length mismatch", stun_bad_length, Kind::Unknown),
            ("turn channel data", channel_data, Kind::Unknown),
            ("disco envelope", disco, Kind::Disco),
            ("other kith envelope", ledger, Kind::Unknown),
            ("empty", Vec::new(), Kind::Unknown),
            ("short", vec![4, 0, 0], Kind::Unknown),
            ("http", b"GET / HTTP/1.1\r\n\r\n".to_vec(), Kind::Unknown),
            ("noise", (0..=255).rev().collect(), Kind::Unknown),
        ];
        for (name, datagram, expected) in cases {
            assert_eq!(classify(datagram), *expected, "{name}");
        }
    }
}
