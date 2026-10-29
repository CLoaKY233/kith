//! What a tunnel does with a packet bigger than its path carries, as a router does
//! (RFC 1191, RFC 8201): split an IPv4 packet that allows it into fragments, or tell the
//! sender the size that fits with ICMP "fragmentation needed" or `ICMPv6` "packet too
//! big", so its path MTU cache shrinks and it resends smaller.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::checksum::{Checksum, checksum};
use crate::field::{bytes, set_bytes, set_u16, slice, u8_at, u16_at};
use crate::ip_protocol;

/// The smallest MTU an IPv6 link may have. `ICMPv6` "packet too big" never reports less:
/// a host would ignore it.
pub const IPV6_MIN_MTU: u16 = 1280;
/// The largest ICMP error for IPv4, as RFC 1812 asks.
const V4_ERROR_MAX: usize = 576;
const V4_HEADER: usize = 20;
const V6_HEADER: usize = 40;
const ICMP_HEADER: usize = 8;
/// How much of the original an IPv4 error quotes, so the error stays within 576 bytes.
const V4_QUOTE_MAX: usize = V4_ERROR_MAX - V4_HEADER - ICMP_HEADER;
/// How much of the original an `ICMPv6` error quotes, so it stays within 1280 bytes.
const V6_QUOTE_MAX: usize = 1280 - V6_HEADER - ICMP_HEADER;

const _: () = assert!(IPV6_MIN_MTU == 1280);
const DONT_FRAGMENT: u16 = 0x4000;
const MORE_FRAGMENTS: u16 = 0x2000;
const OFFSET_MASK: u16 = 0x1fff;

/// What to do with a packet bigger than the path carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oversize {
    /// IPv4 without options that may be fragmented: split it with [`fragment_v4`].
    Fragment,
    /// Tell the sender with [`too_big_reply`], then drop it.
    Reply,
    /// Drop it silently: an ICMP error itself, from an unusable source, or malformed.
    Drop,
}

/// Decides what to do with `packet`, which is too big for the path.
pub fn oversize(packet: &[u8]) -> Oversize {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= V4_HEADER => {
            let source = IpAddr::V4(Ipv4Addr::from(bytes::<4>(packet, 12)));
            let header = usize::from(u8_at(packet, 0) & 0x0f).saturating_mul(4);
            let error = u8_at(packet, 9) == ip_protocol::ICMP
                && matches!(u8_at(packet, header), 3 | 4 | 5 | 11 | 12);
            if error || !usable_source(source) {
                Oversize::Drop
            } else if u16_at(packet, 6) & DONT_FRAGMENT == 0 && header == V4_HEADER {
                Oversize::Fragment
            } else {
                Oversize::Reply
            }
        }
        Some(6) if packet.len() >= V6_HEADER => {
            let source = IpAddr::V6(Ipv6Addr::from(bytes::<16>(packet, 8)));
            let error = u8_at(packet, 6) == ip_protocol::ICMPV6 && u8_at(packet, V6_HEADER) < 128;
            if error || !usable_source(source) {
                Oversize::Drop
            } else {
                Oversize::Reply
            }
        }
        _ => Oversize::Drop,
    }
}

fn usable_source(source: IpAddr) -> bool {
    let broadcast = matches!(source, IpAddr::V4(v4) if v4.is_broadcast());
    !(source.is_unspecified() || source.is_multicast() || source.is_loopback() || broadcast)
}

/// Writes into `out` the ICMP error telling `packet`'s sender that only `mtu` bytes fit,
/// from `packet`'s destination to its source, and returns its length. For IPv6 the MTU
/// reported is at least [`IPV6_MIN_MTU`]. `None` if `out` is too small or `packet` is
/// neither IPv4 nor IPv6.
pub fn too_big_reply(packet: &[u8], mtu: u16, out: &mut [u8]) -> Option<usize> {
    match packet.first()? >> 4 {
        4 if packet.len() >= V4_HEADER => reply_v4(packet, mtu, out),
        6 if packet.len() >= V6_HEADER => reply_v6(packet, mtu, out),
        _ => None,
    }
}

fn reply_v4(packet: &[u8], mtu: u16, out: &mut [u8]) -> Option<usize> {
    let quoted = packet.len().min(V4_QUOTE_MAX);
    let total = quoted.saturating_add(V4_HEADER + ICMP_HEADER);
    let out = out.get_mut(..total)?;
    out.fill(0);
    set_bytes(out, 0, [0x45]);
    set_u16(out, 2, u16::try_from(total).ok()?);
    set_bytes(out, 8, [64, ip_protocol::ICMP]);
    set_bytes(out, 12, bytes::<4>(packet, 16));
    set_bytes(out, 16, bytes::<4>(packet, 12));
    let header_sum = checksum(slice(out, 0, V4_HEADER));
    set_u16(out, 10, header_sum);
    let icmp = out.get_mut(V4_HEADER..)?;
    set_bytes(icmp, 0, [3, 4]);
    set_u16(icmp, 6, mtu);
    icmp.get_mut(ICMP_HEADER..)?
        .copy_from_slice(packet.get(..quoted)?);
    let icmp_sum = checksum(icmp);
    set_u16(icmp, 2, icmp_sum);
    Some(total)
}

fn reply_v6(packet: &[u8], mtu: u16, out: &mut [u8]) -> Option<usize> {
    let quoted = packet.len().min(V6_QUOTE_MAX);
    let payload = quoted.saturating_add(ICMP_HEADER);
    let total = payload.saturating_add(V6_HEADER);
    let out = out.get_mut(..total)?;
    out.fill(0);
    set_bytes(out, 0, [0x60]);
    set_u16(out, 4, u16::try_from(payload).ok()?);
    set_bytes(out, 6, [ip_protocol::ICMPV6, 64]);
    set_bytes(out, 8, bytes::<16>(packet, 24));
    set_bytes(out, 24, bytes::<16>(packet, 8));
    let (header, icmp) = out.split_at_mut(V6_HEADER);
    set_bytes(icmp, 0, [2]);
    set_bytes(icmp, 4, u32::from(mtu.max(IPV6_MIN_MTU)).to_be_bytes());
    icmp.get_mut(ICMP_HEADER..)?
        .copy_from_slice(packet.get(..quoted)?);
    let mut sum = Checksum::new();
    sum.add_bytes(slice(header, 8, V6_HEADER));
    sum.add_value(u64::try_from(payload).ok()?);
    sum.add_value(u64::from(ip_protocol::ICMPV6));
    sum.add_bytes(icmp);
    set_u16(icmp, 2, sum.finish());
    Some(total)
}

/// Splits the IPv4 packet `packet` into fragments of at most `mtu` bytes, writing each
/// into `scratch` and calling `emit` with it, and returns how many it made. `packet` must
/// be one [`oversize`] answered [`Oversize::Fragment`] for; a fragment of a fragment
/// keeps its offset and more-fragments bit. Makes none if `mtu` leaves no room for data.
pub fn fragment_v4(
    packet: &[u8],
    mtu: u16,
    scratch: &mut [u8],
    mut emit: impl FnMut(&[u8]),
) -> usize {
    let end = usize::from(u16_at(packet, 2)).min(packet.len());
    let data = slice(packet, V4_HEADER, end);
    let room = usize::from(mtu).saturating_sub(V4_HEADER) & !7;
    if room == 0 || data.is_empty() {
        return 0;
    }
    let flags = u16_at(packet, 6);
    let base = usize::from(flags & OFFSET_MASK);
    let more_after = flags & MORE_FRAGMENTS != 0;
    let mut made: usize = 0;
    let mut offset: usize = 0;
    for chunk in data.chunks(room) {
        let len = V4_HEADER.saturating_add(chunk.len());
        let Some(out) = scratch.get_mut(..len) else {
            return made;
        };
        let Some((header, body)) = out.split_at_mut_checked(V4_HEADER) else {
            return made;
        };
        header.copy_from_slice(slice(packet, 0, V4_HEADER));
        body.copy_from_slice(chunk);
        offset = offset.saturating_add(chunk.len());
        let more = offset < data.len() || more_after;
        let start = base.saturating_add(offset.saturating_sub(chunk.len()) / 8);
        let field = (u16::try_from(start).unwrap_or(OFFSET_MASK) & OFFSET_MASK)
            | if more { MORE_FRAGMENTS } else { 0 };
        set_u16(out, 2, u16::try_from(len).unwrap_or(u16::MAX));
        set_u16(out, 6, field);
        set_u16(out, 10, 0);
        let sum = checksum(slice(out, 0, V4_HEADER));
        set_u16(out, 10, sum);
        emit(out);
        made = made.saturating_add(1);
    }
    made
}
