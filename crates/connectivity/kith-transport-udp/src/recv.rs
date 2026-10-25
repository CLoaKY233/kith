//! Incoming datagram batches.
//!
//! One receive syscall fills up to [`quinn_udp::BATCH_SIZE`] slots, and with GRO the kernel may
//! pack several datagrams from one source into a slot at a fixed stride.
//! [`RecvBatch::datagrams`] hides both levels.

use std::io::IoSliceMut;
use std::net::{IpAddr, SocketAddr};

use quinn_udp::{BATCH_SIZE, RecvMeta};

use crate::UdpLink;

/// Largest slot: a GRO super-buffer never exceeds the 64 KiB IP payload limit.
pub const MAX_SLOT_LEN: usize = 64 * 1024;

/// Largest datagram the compact preset expects, an Ethernet MTU. Larger ones are truncated.
pub(crate) const PATH_DATAGRAM_LEN: usize = 1500;

/// Memory budget of [`RecvBatch::compact`].
pub(crate) const COMPACT_BUDGET: usize = 256 * 1024;

/// One received datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Datagram<'a> {
    /// Sender; IPv4-mapped addresses are reported as IPv4.
    pub source: SocketAddr,
    /// Local destination address, where the platform reports it.
    pub destination_ip: Option<IpAddr>,
    /// UDP payload.
    pub payload: &'a [u8],
}

/// Reusable receive buffers for [`UdpLink::recv_batch`]. Receiving never allocates.
#[derive(Debug)]
pub struct RecvBatch {
    storage: Box<[u8]>,
    slot_len: usize,
    meta: [RecvMeta; BATCH_SIZE],
    filled: usize,
}

impl RecvBatch {
    /// `slots` slots of `slot_len` bytes, clamped to `1..=BATCH_SIZE` and `1..=MAX_SLOT_LEN`.
    pub fn with_capacity(slots: usize, slot_len: usize) -> Self {
        let slots = slots.clamp(1, BATCH_SIZE);
        let slot_len = slot_len.clamp(1, MAX_SLOT_LEN);
        Self {
            storage: vec![0; slots * slot_len].into_boxed_slice(),
            slot_len,
            meta: [RecvMeta::default(); BATCH_SIZE],
            filled: 0,
        }
    }

    /// Router preset: at most 256 KiB, slots sized for `link`'s GRO.
    pub fn compact(link: &UdpLink) -> Self {
        let slot_len = link
            .gro_segments()
            .saturating_mul(PATH_DATAGRAM_LEN)
            .clamp(PATH_DATAGRAM_LEN, MAX_SLOT_LEN);
        Self::with_capacity(COMPACT_BUDGET / slot_len, slot_len)
    }

    /// Server preset: `BATCH_SIZE` slots of 64 KiB (2 MiB on Linux, 64 KiB elsewhere).
    pub fn server() -> Self {
        Self::with_capacity(BATCH_SIZE, MAX_SLOT_LEN)
    }

    /// Datagrams from the most recent receive, in arrival order.
    pub fn datagrams(&self) -> impl Iterator<Item = Datagram<'_>> {
        self.meta
            .iter()
            .take(self.filled)
            .zip(self.storage.chunks_exact(self.slot_len))
            .flat_map(|(meta, slot)| {
                let payload = slot.get(..meta.len).unwrap_or_default();
                let source = SocketAddr::new(meta.addr.ip().to_canonical(), meta.addr.port());
                let destination_ip = meta.dst_ip.map(|ip| ip.to_canonical());
                payload
                    .chunks(meta.stride.max(1))
                    .map(move |payload| Datagram {
                        source,
                        destination_ip,
                        payload,
                    })
            })
    }

    /// Clears the previous contents and lends out the slots for one syscall.
    pub(crate) fn buffers(&mut self) -> (BufferSet<'_>, &mut [RecvMeta]) {
        self.filled = 0;
        let mut chunks = self.storage.chunks_exact_mut(self.slot_len);
        let count = chunks.len();
        let slices = core::array::from_fn(|_| IoSliceMut::new(chunks.next().unwrap_or_default()));
        (BufferSet { slices, count }, &mut self.meta)
    }

    /// Records how many slots were filled and returns the datagram count.
    pub(crate) fn set_filled(&mut self, slots: usize) -> usize {
        self.filled = slots.min(BATCH_SIZE);
        self.meta
            .iter()
            .take(self.filled)
            .map(|meta| meta.len.div_ceil(meta.stride.max(1)))
            .sum()
    }
}

/// I/O slices over the slots of a [`RecvBatch`].
pub(crate) struct BufferSet<'a> {
    slices: [IoSliceMut<'a>; BATCH_SIZE],
    count: usize,
}

impl<'a> BufferSet<'a> {
    /// Only the real slots: an empty slice would receive a datagram truncated to nothing.
    pub(crate) fn slots(&mut self) -> &mut [IoSliceMut<'a>] {
        let count = self.count;
        self.slices.get_mut(..count).unwrap_or_default()
    }
}
