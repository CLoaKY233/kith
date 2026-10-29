//! Putting IP fragments back together so the firewall can judge a whole packet: a later
//! fragment carries no ports, so on its own it can never match a flow or a rule. The
//! firewall sees the reassembled packet; the original fragments are what is delivered,
//! so nothing bigger than the sender's packets reaches the OS, which reassembles them
//! itself.
//!
//! Bounded as Linux's `nf_defrag` is: few datagrams at once, little memory, a short
//! timeout, and a datagram whose fragments overlap is thrown away whole (RFC 5722).

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

/// How long the fragments of one datagram may take to arrive.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
/// The most datagrams being put back together at once.
const MAX_PENDING: usize = 64;
/// The most fragment bytes held at once.
const MAX_BYTES: usize = 256 * 1024;
/// The largest datagram IP can carry.
const MAX_DATAGRAM: usize = 65_535;
const V4_HEADER: usize = 20;
const V6_HEADER: usize = 40;
const V6_FRAGMENT: u8 = 44;
const V6_FRAGMENT_HEADER: usize = 8;

/// A datagram whose fragments have all arrived.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Complete {
    /// The datagram as it was before it was split, for the firewall.
    pub(crate) whole: Vec<u8>,
    /// The fragments as they arrived, to deliver.
    pub(crate) parts: Vec<Vec<u8>>,
}

/// One fragment, parsed.
struct Fragment<'a> {
    key: Key,
    /// The bytes before the fragment's data that start the reassembled datagram.
    header: &'a [u8],
    /// Where its data starts in the datagram's payload.
    offset: usize,
    data: &'a [u8],
    last: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    source: IpAddr,
    destination: IpAddr,
    id: u32,
    protocol: u8,
}

struct Pending {
    started: Duration,
    /// The unfragmentable part, from the fragment at offset zero.
    header: Option<Vec<u8>>,
    /// Data by offset.
    pieces: BTreeMap<usize, Vec<u8>>,
    /// The payload's length, once the last fragment came.
    total: Option<usize>,
    parts: Vec<Vec<u8>>,
    bytes: usize,
}

/// Datagrams being put back together.
#[derive(Default)]
pub(crate) struct Reassembly {
    pending: BTreeMap<Key, Pending>,
    bytes: usize,
}

/// Whether `packet` is an IPv4 or IPv6 fragment.
pub(crate) fn is_fragment(packet: &[u8]) -> bool {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) => packet
            .get(6..8)
            .and_then(|field| <[u8; 2]>::try_from(field).ok())
            .is_some_and(|field| u16::from_be_bytes(field) & 0x3fff != 0),
        Some(6) => packet.get(6) == Some(&V6_FRAGMENT),
        _ => false,
    }
}

impl Reassembly {
    /// Takes in a fragment at `now`; returns the datagram once all of it has come, and
    /// `None` meanwhile or when the fragment is malformed, overlaps one already held, or
    /// would make the datagram too big. Datagrams older than [`TIMEOUT`] are forgotten.
    pub(crate) fn add(&mut self, packet: &[u8], now: Duration) -> Option<Complete> {
        self.expire(now);
        let fragment = parse(packet)?;
        let end = fragment.offset.checked_add(fragment.data.len())?;
        if end.saturating_add(fragment.header.len()) > MAX_DATAGRAM {
            self.forget(&fragment.key);
            return None;
        }
        if !self.pending.contains_key(&fragment.key) {
            self.make_room(packet.len());
        }
        let pending = self.pending.entry(fragment.key).or_insert_with(|| Pending {
            started: now,
            header: None,
            pieces: BTreeMap::new(),
            total: None,
            parts: Vec::new(),
            bytes: 0,
        });
        let overlaps = pending
            .pieces
            .range(..end)
            .next_back()
            .is_some_and(|(start, data)| start.saturating_add(data.len()) > fragment.offset);
        let past_end = pending.total.is_some_and(|total| end > total)
            || (fragment.last
                && pending
                    .pieces
                    .keys()
                    .next_back()
                    .is_some_and(|start| *start >= end));
        if overlaps || past_end || (fragment.last && pending.total.is_some()) {
            self.forget(&fragment.key);
            return None;
        }
        if fragment.offset == 0 {
            pending.header = Some(fragment.header.to_vec());
        }
        if fragment.last {
            pending.total = Some(end);
        }
        pending
            .pieces
            .insert(fragment.offset, fragment.data.to_vec());
        pending.parts.push(packet.to_vec());
        pending.bytes = pending.bytes.saturating_add(packet.len());
        self.bytes = self.bytes.saturating_add(packet.len());
        let key = fragment.key;
        let whole = assemble(self.pending.get(&key)?, key.protocol)?;
        let done = self.pending.remove(&key)?;
        self.bytes = self.bytes.saturating_sub(done.bytes);
        Some(Complete {
            whole,
            parts: done.parts,
        })
    }

    fn expire(&mut self, now: Duration) {
        let stale: Vec<Key> = self
            .pending
            .iter()
            .filter(|(_, pending)| now.saturating_sub(pending.started) >= TIMEOUT)
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.forget(&key);
        }
    }

    /// Forgets the oldest datagrams until one more of `incoming` bytes fits.
    fn make_room(&mut self, incoming: usize) {
        while self.pending.len() >= MAX_PENDING || self.bytes.saturating_add(incoming) > MAX_BYTES {
            let Some(oldest) = self
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.started)
                .map(|(key, _)| *key)
            else {
                return;
            };
            self.forget(&oldest);
        }
    }

    fn forget(&mut self, key: &Key) {
        if let Some(pending) = self.pending.remove(key) {
            self.bytes = self.bytes.saturating_sub(pending.bytes);
        }
    }
}

/// The datagram, if every byte of its payload is held; `protocol` is what an IPv6
/// fragment header said follows it.
fn assemble(pending: &Pending, protocol: u8) -> Option<Vec<u8>> {
    let total = pending.total?;
    let header = pending.header.as_ref()?;
    let mut payload = Vec::with_capacity(total);
    for (start, data) in &pending.pieces {
        if *start != payload.len() {
            return None;
        }
        payload.extend_from_slice(data);
    }
    if payload.len() != total {
        return None;
    }
    let mut whole = header.clone();
    whole.extend_from_slice(&payload);
    let length = u16::try_from(whole.len()).ok()?;
    if whole.first()? >> 4 == 4 {
        whole.get_mut(2..4)?.copy_from_slice(&length.to_be_bytes());
        whole.get_mut(6..8)?.copy_from_slice(&[0, 0]);
        whole.get_mut(10..12)?.copy_from_slice(&[0, 0]);
        let sum = header_checksum(whole.get(..header.len())?);
        whole.get_mut(10..12)?.copy_from_slice(&sum.to_be_bytes());
    } else {
        let payload_len = u16::try_from(total).ok()?;
        whole
            .get_mut(4..6)?
            .copy_from_slice(&payload_len.to_be_bytes());
        *whole.get_mut(6)? = protocol;
    }
    Some(whole)
}

fn parse(packet: &[u8]) -> Option<Fragment<'_>> {
    match packet.first()? >> 4 {
        4 => {
            let header_len = usize::from(packet.first()? & 0x0f).checked_mul(4)?;
            let total = usize::from(u16::from_be_bytes(packet.get(2..4)?.try_into().ok()?));
            if header_len < V4_HEADER || total < header_len || total > packet.len() {
                return None;
            }
            let field = u16::from_be_bytes(packet.get(6..8)?.try_into().ok()?);
            let source: [u8; 4] = packet.get(12..16)?.try_into().ok()?;
            let destination: [u8; 4] = packet.get(16..20)?.try_into().ok()?;
            let data = packet.get(header_len..total)?;
            let last = field & 0x2000 == 0;
            if !last && data.len() % 8 != 0 {
                return None;
            }
            Some(Fragment {
                key: Key {
                    source: IpAddr::from(source),
                    destination: IpAddr::from(destination),
                    id: u32::from(u16::from_be_bytes(packet.get(4..6)?.try_into().ok()?)),
                    protocol: *packet.get(9)?,
                },
                header: packet.get(..header_len)?,
                offset: usize::from(field & 0x1fff).checked_mul(8)?,
                data,
                last,
            })
        }
        6 => {
            if *packet.get(6)? != V6_FRAGMENT {
                return None;
            }
            let payload_len = usize::from(u16::from_be_bytes(packet.get(4..6)?.try_into().ok()?));
            let end = V6_HEADER.checked_add(payload_len)?;
            let fragment_header = packet.get(V6_HEADER..V6_HEADER + V6_FRAGMENT_HEADER)?;
            let next = *fragment_header.first()?;
            let field = u16::from_be_bytes(fragment_header.get(2..4)?.try_into().ok()?);
            let data = packet.get(V6_HEADER + V6_FRAGMENT_HEADER..end)?;
            let last = field & 1 == 0;
            if !last && data.len() % 8 != 0 {
                return None;
            }
            let source: [u8; 16] = packet.get(8..24)?.try_into().ok()?;
            let destination: [u8; 16] = packet.get(24..40)?.try_into().ok()?;
            Some(Fragment {
                key: Key {
                    source: IpAddr::from(source),
                    destination: IpAddr::from(destination),
                    id: u32::from_be_bytes(fragment_header.get(4..8)?.try_into().ok()?),
                    protocol: next,
                },
                header: packet.get(..V6_HEADER)?,
                offset: usize::from(field >> 3).checked_mul(8)?,
                data,
                last,
            })
        }
        _ => None,
    }
}

/// The IPv4 header checksum of `header`, whose checksum field is zero.
fn header_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for word in header.chunks(2) {
        let high = u32::from(word.first().copied().unwrap_or(0));
        let low = u32::from(word.get(1).copied().unwrap_or(0));
        sum = sum.wrapping_add((high << 8) | low);
    }
    while sum > 0xffff {
        sum = (sum & 0xffff).wrapping_add(sum >> 16);
    }
    !u16::try_from(sum).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::{MAX_PENDING, Reassembly, TIMEOUT, is_fragment};

    /// A UDP datagram of `len` bytes with IP ID `id`, then its fragments of at most `mtu`.
    fn split(len: usize, id: u16, mtu: u16) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut packet = vec![0_u8; len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&u16::try_from(len).unwrap().to_be_bytes());
        packet[4..6].copy_from_slice(&id.to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&[100, 64, 0, 2]);
        packet[16..20].copy_from_slice(&[100, 64, 0, 3]);
        for (index, byte) in packet[20..].iter_mut().enumerate() {
            *byte = u8::try_from(index % 253).unwrap();
        }
        kith_packet::Ipv4Packet::new_checked(&mut packet[..])
            .unwrap()
            .fill_checksum();
        let mut parts = Vec::new();
        let mut scratch = vec![0; 1500];
        kith_packet::fragment_v4(&packet, mtu, &mut scratch, |part| parts.push(part.to_vec()));
        (packet, parts)
    }

    fn at(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn fragments_in_any_order_give_back_the_datagram_and_themselves() {
        let (packet, parts) = split(3000, 7, 1000);
        assert!(parts.iter().all(|part| is_fragment(part)) && !is_fragment(&packet));
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
            let mut table = Reassembly::default();
            let mut done = None;
            for index in order {
                assert!(done.is_none(), "finished early");
                done = table.add(&parts[index], at(0));
            }
            let done = done.unwrap();
            assert_eq!(done.whole, packet);
            assert_eq!(done.parts.len(), parts.len());
            assert!(table.pending.is_empty() && table.bytes == 0);
        }
    }

    #[test]
    fn an_overlap_a_repeat_or_a_late_fragment_loses_the_datagram() {
        let (_, parts) = split(3000, 8, 1000);
        let mut repeated = Reassembly::default();
        assert!(repeated.add(&parts[0], at(0)).is_none());
        assert!(repeated.add(&parts[0], at(0)).is_none());
        assert!(
            repeated.pending.is_empty(),
            "a repeated fragment is an overlap"
        );
        for part in &parts[1..] {
            assert!(repeated.add(part, at(0)).is_none(), "nothing of it is kept");
        }

        let mut late = Reassembly::default();
        assert!(late.add(&parts[0], at(0)).is_none());
        let rest: Vec<_> = parts[1..]
            .iter()
            .map(|part| late.add(part, TIMEOUT))
            .collect();
        assert!(
            rest.iter().all(Option::is_none),
            "a datagram expires after its timeout"
        );
    }

    #[test]
    fn the_table_stays_small_whatever_arrives() {
        let mut table = Reassembly::default();
        for id in 0..1000_u16 {
            let (_, parts) = split(3000, id, 1000);
            assert!(table.add(&parts[0], at(0)).is_none());
            assert!(table.pending.len() <= MAX_PENDING);
            assert!(table.bytes <= super::MAX_BYTES);
        }
        let (packet, parts) = split(3000, 5000, 1000);
        let done = parts.iter().find_map(|part| table.add(part, at(0)));
        assert_eq!(
            done.map(|done| done.whole),
            Some(packet),
            "new datagrams still get through"
        );
    }

    #[test]
    fn ipv6_fragments_are_put_back_together_with_their_next_header() {
        let data: Vec<u8> = (0..2000_u32)
            .map(|n| u8::try_from(n % 251).unwrap())
            .collect();
        let fragment = |offset: usize, chunk: &[u8], more: bool| {
            let mut packet = vec![0_u8; 48];
            packet[0] = 0x60;
            packet[4..6].copy_from_slice(&u16::try_from(8 + chunk.len()).unwrap().to_be_bytes());
            packet[6] = 44;
            packet[7] = 64;
            packet[8] = 0xfd;
            packet[24] = 0xfd;
            packet[40] = 17;
            let field = u16::try_from(offset).unwrap() | u16::from(more);
            packet[42..44].copy_from_slice(&field.to_be_bytes());
            packet[44..48].copy_from_slice(&99_u32.to_be_bytes());
            packet.extend_from_slice(chunk);
            packet
        };
        let first = fragment(0, &data[..1200], true);
        let second = fragment(1200, &data[1200..], false);
        let mut table = Reassembly::default();
        assert!(table.add(&second, at(0)).is_none());
        let done = table.add(&first, at(0)).unwrap();
        assert_eq!(
            done.whole[6], 17,
            "the next header is the fragment header's"
        );
        assert_eq!(
            usize::from(u16::from_be_bytes([done.whole[4], done.whole[5]])),
            2000
        );
        assert_eq!(&done.whole[40..], &data[..]);
    }

    proptest! {
        #[test]
        fn anything_at_all_never_panics_or_grows_the_table(
            packets in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..200), 0..50),
        ) {
            let mut table = Reassembly::default();
            for (index, packet) in packets.iter().enumerate() {
                drop(table.add(packet, at(u64::try_from(index).unwrap())));
                prop_assert!(table.pending.len() <= MAX_PENDING);
            }
        }
    }
}
