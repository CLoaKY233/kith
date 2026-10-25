//! Outgoing datagram batches and their coalescing into offloaded sends.
//!
//! Traffic to a peer is mostly runs of full-MTU datagrams. GSO sends a run in one syscall; the
//! kernel requires equal segments except a shorter last one.

use std::net::SocketAddr;

use quinn_udp::Transmit;

/// Largest UDP payload that fits in an IPv4 packet without jumbograms.
pub(crate) const MAX_DATAGRAM_LEN: usize = 65_507;

/// Largest total GSO payload: the segments share one IP packet's payload limit.
const MAX_GSO_PAYLOAD: usize = MAX_DATAGRAM_LEN;

/// One queued datagram: where it goes and where its bytes sit in [`SendBatch::bytes`].
#[derive(Debug, Clone, Copy)]
struct Entry {
    destination: SocketAddr,
    start: usize,
    len: usize,
}

impl Entry {
    fn end(&self) -> usize {
        self.start + self.len
    }
}

/// A reusable batch of outgoing datagrams.
///
/// Payloads sit back to back so a coalesced run is a sub-slice; [`clear`](Self::clear) keeps
/// capacity.
#[derive(Debug, Default)]
pub struct SendBatch {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

impl SendBatch {
    /// Creates an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty batch with room for `datagrams` datagrams totalling `bytes` bytes.
    pub fn with_capacity(datagrams: usize, bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            entries: Vec::with_capacity(datagrams),
        }
    }

    /// Queues `payload` for `destination`.
    pub fn push(&mut self, destination: SocketAddr, payload: &[u8]) -> Result<(), crate::UdpError> {
        if payload.len() > MAX_DATAGRAM_LEN {
            return Err(crate::UdpError::DatagramTooLarge { len: payload.len() });
        }
        self.entries.push(Entry {
            destination,
            start: self.bytes.len(),
            len: payload.len(),
        });
        self.bytes.extend_from_slice(payload);
        Ok(())
    }

    /// Removes every datagram, keeping the allocated capacity.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.entries.clear();
    }

    /// Number of queued datagrams.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no datagram is queued.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The batch as transmits of at most `max_segments` datagrams each.
    pub(crate) fn transmits(&self, max_segments: usize) -> Transmits<'_> {
        Transmits {
            batch: self,
            next: 0,
            max_segments: max_segments.max(1),
        }
    }
}

/// Iterator returned by [`SendBatch::transmits`].
#[derive(Debug)]
pub(crate) struct Transmits<'a> {
    batch: &'a SendBatch,
    next: usize,
    max_segments: usize,
}

impl<'a> Iterator for Transmits<'a> {
    type Item = Transmit<'a>;

    fn next(&mut self) -> Option<Transmit<'a>> {
        let (first, rest) = self.batch.entries.get(self.next..)?.split_first()?;
        let segment_size = first.len;
        let mut count = 1;
        let mut end = first.end();
        // A zero-length datagram cannot be a GSO segment, so it always goes alone.
        if segment_size > 0 {
            for entry in rest.iter().take(self.max_segments - 1) {
                let joins = entry.destination == first.destination
                    && (1..=segment_size).contains(&entry.len)
                    && entry.end() - first.start <= MAX_GSO_PAYLOAD;
                if !joins {
                    break;
                }
                count += 1;
                end = entry.end();
                if entry.len < segment_size {
                    break;
                }
            }
        }
        self.next += count;
        // Entries are contiguous and in bounds by construction in `push`.
        let contents = self.batch.bytes.get(first.start..end)?;
        Some(Transmit {
            destination: first.destination,
            ecn: None,
            contents,
            segment_size: (count > 1).then_some(segment_size),
            src_ip: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use proptest::prelude::*;

    use super::{MAX_GSO_PAYLOAD, SendBatch};

    fn destination(index: u8) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::new(192, 0, 2, index), 51820))
    }

    /// Few destinations and few distinct sizes, so long same-size runs are common.
    fn datagrams() -> impl Strategy<Value = Vec<(u8, Vec<u8>)>> {
        let size = prop_oneof![
            Just(0_usize),
            Just(1),
            Just(1200),
            Just(1452),
            0_usize..1500
        ];
        let datagram =
            (0_u8..3, size, any::<u8>()).prop_map(|(dest, len, fill)| (dest, vec![fill; len]));
        prop::collection::vec(datagram, 0..200)
    }

    proptest! {
        #[test]
        fn coalesced_transmits_split_back_into_the_original_sequence(
            input in datagrams(),
            max_segments in 1_usize..70,
        ) {
            let mut batch = SendBatch::new();
            for (dest, payload) in &input {
                batch.push(destination(*dest), payload).unwrap();
            }

            let mut output = Vec::new();
            for transmit in batch.transmits(max_segments) {
                let Some(size) = transmit.segment_size else {
                    output.push((transmit.destination, transmit.contents.to_vec()));
                    continue;
                };
                let segments: Vec<&[u8]> = transmit.contents.chunks(size).collect();
                prop_assert!(segments.len() >= 2 && segments.len() <= max_segments);
                prop_assert!(transmit.contents.len() <= MAX_GSO_PAYLOAD);
                for segment in segments {
                    output.push((transmit.destination, segment.to_vec()));
                }
            }

            let expected: Vec<_> = input
                .into_iter()
                .map(|(dest, payload)| (destination(dest), payload))
                .collect();
            prop_assert_eq!(output, expected);
        }
    }
}
