//! Lowering the maximum segment size a TCP connection announces, so its packets fit a
//! tunnel whose underlying path is smaller than the tunnel's MTU.
//!
//! WireGuard adds 60 bytes over IPv4 and 80 over IPv6, and nothing tells the sender when
//! the path in between drops what is too big: many mobile networks carry no more than
//! 1280 bytes, the IPv6 minimum, and drop larger packets or their fragments silently.
//! TCP is the only protocol a middlebox can fix, by rewriting the MSS option of the SYN
//! and SYN-ACK so neither side ever sends a segment larger than fits.

use crate::checksum::update_bytes;
use crate::field::{u8_at, u16_at};
use crate::ip_protocol as protocol;

const MSS_KIND: u8 = 2;
const MSS_LEN: u8 = 4;
const SYN: u8 = 0x02;

/// Where a TCP SYN's MSS value sits in an IP packet, and the largest MSS that fits `mtu`.
struct Found {
    tcp: usize,
    value: usize,
    max: u16,
}

/// Whether `packet` is a TCP SYN announcing a segment size too big for `mtu`.
pub fn needs_mss_clamp(packet: &[u8], mtu: u16) -> bool {
    find(packet, mtu).is_some_and(|found| u16_at(packet, found.value) > found.max)
}

/// Lowers the MSS of a TCP SYN in the IP packet `packet` so a full segment fits in an IP
/// packet of `mtu` bytes, and returns whether it did. The TCP checksum is updated
/// incrementally, so a checksum left partial for offload stays correct. Anything else
/// is left alone: other protocols, segments without SYN or without an MSS option, an
/// MSS already small enough, IPv4 fragments after the first, IPv6 with extension
/// headers, and anything malformed.
pub fn clamp_mss(packet: &mut [u8], mtu: u16) -> bool {
    let Some(found) = find(packet, mtu) else {
        return false;
    };
    let old = u16_at(packet, found.value);
    if old <= found.max {
        return false;
    }
    let checksum_at = found.tcp.saturating_add(16);
    let odd = found.value.saturating_sub(found.tcp) % 2;
    let start = found.value.saturating_sub(odd);
    let end = found.value.saturating_add(2).saturating_add(odd);
    let mut before = [0_u8; 4];
    let mut after = [0_u8; 4];
    let width = end.saturating_sub(start);
    let (Some(old_words), Some(before)) = (packet.get(start..end), before.get_mut(..width)) else {
        return false;
    };
    before.copy_from_slice(old_words);
    let Some(field) = packet.get_mut(found.value..found.value.saturating_add(2)) else {
        return false;
    };
    field.copy_from_slice(&found.max.to_be_bytes());
    let (Some(new_words), Some(after)) = (packet.get(start..end), after.get_mut(..width)) else {
        return false;
    };
    after.copy_from_slice(new_words);
    let updated = update_bytes(u16_at(packet, checksum_at), before, after);
    if let Some(field) = packet.get_mut(checksum_at..checksum_at.saturating_add(2)) {
        field.copy_from_slice(&updated.to_be_bytes());
    }
    true
}

fn find(packet: &[u8], mtu: u16) -> Option<Found> {
    let (tcp, overhead) = match packet.first()? >> 4 {
        4 => {
            let header = usize::from(packet.first()? & 0x0f).checked_mul(4)?;
            let fragment_offset = u16_at(packet, 6) & 0x1fff;
            if header < 20 || fragment_offset != 0 || u8_at(packet, 9) != protocol::TCP {
                return None;
            }
            (header, 40_u16)
        }
        6 => {
            if u8_at(packet, 6) != protocol::TCP {
                return None;
            }
            (40, 60_u16)
        }
        _ => return None,
    };
    let offset_byte = *packet.get(tcp.checked_add(12)?)?;
    let flags = *packet.get(tcp.checked_add(13)?)?;
    let header = usize::from(offset_byte >> 4).checked_mul(4)?;
    let end = tcp.checked_add(header)?;
    if flags & SYN == 0 || header < 20 || end > packet.len() {
        return None;
    }
    let mut at = tcp.checked_add(20)?;
    while at < end {
        match *packet.get(at)? {
            0 => return None,
            1 => at = at.checked_add(1)?,
            kind => {
                let len = *packet.get(at.checked_add(1)?)?;
                if len < 2 || at.checked_add(usize::from(len))? > end {
                    return None;
                }
                if kind == MSS_KIND {
                    if len != MSS_LEN {
                        return None;
                    }
                    return Some(Found {
                        tcp,
                        value: at.checked_add(2)?,
                        max: mtu.checked_sub(overhead)?,
                    });
                }
                at = at.checked_add(usize::from(len))?;
            }
        }
    }
    None
}
