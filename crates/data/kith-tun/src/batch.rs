//! Preallocated packet buffers. Each packet starts at [`HEADROOM`] so tun-rs
//! and upstream crates can write headers in front of it without copying.

#[cfg(target_os = "linux")]
use tun_rs::GROTable;

/// Bytes reserved before each packet: fits the virtio-net and WireGuard headers.
pub const HEADROOM: usize = 16;

/// Maximum packets per batch.
pub const BATCH_SIZE: usize = 128;

/// Largest per-packet capacity [`PacketBatch::new`] accepts.
pub const MAX_PACKET_CAPACITY: usize = 65_535;

/// Size of the scratch buffer one GSO superpacket is read into.
#[cfg(target_os = "linux")]
const GSO_SCRATCH_LEN: usize = tun_rs::VIRTIO_NET_HDR_LEN + MAX_PACKET_CAPACITY;

#[cfg(target_os = "linux")]
const _: () = assert!(HEADROOM >= tun_rs::VIRTIO_NET_HDR_LEN);
#[cfg(target_os = "linux")]
const _: () = assert!(BATCH_SIZE == tun_rs::IDEAL_BATCH_SIZE);
#[expect(
    clippy::as_conversions,
    reason = "widening u16 to usize in a const, where `From` is not callable"
)]
const _: () = assert!(MAX_PACKET_CAPACITY.div_ceil(crate::MIN_MTU as usize) <= BATCH_SIZE);

/// Why a [`PacketBatch`] operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BatchError {
    /// The requested per-packet capacity is zero or above [`MAX_PACKET_CAPACITY`].
    #[error("packet capacity {capacity} is outside 1..={MAX_PACKET_CAPACITY}")]
    CapacityOutOfRange {
        /// The rejected capacity.
        capacity: usize,
    },
    /// The batch already holds [`BATCH_SIZE`] packets.
    #[error("batch already holds {BATCH_SIZE} packets")]
    Full,
    /// The packet does not fit in one slot.
    #[error("packet of {len} bytes exceeds the slot capacity of {capacity}")]
    PacketTooLarge {
        /// Length of the rejected packet.
        len: usize,
        /// The batch's per-packet capacity.
        capacity: usize,
    },
}

/// Up to [`BATCH_SIZE`] packets in buffers allocated once.
pub struct PacketBatch {
    buffers: Vec<Vec<u8>>,
    sizes: Vec<usize>,
    len: usize,
    packet_capacity: usize,
    #[cfg(target_os = "linux")]
    gso_scratch: Vec<u8>,
    #[cfg(target_os = "linux")]
    gro_table: GROTable,
}

impl core::fmt::Debug for PacketBatch {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PacketBatch")
            .field("len", &self.len)
            .field("packet_capacity", &self.packet_capacity)
            .finish_non_exhaustive()
    }
}

impl PacketBatch {
    /// Allocates all slots; `packet_capacity` must be at least the MTU for reads.
    pub fn new(packet_capacity: usize) -> Result<Self, BatchError> {
        if !(1..=MAX_PACKET_CAPACITY).contains(&packet_capacity) {
            return Err(BatchError::CapacityOutOfRange {
                capacity: packet_capacity,
            });
        }
        let full_len = HEADROOM + packet_capacity;
        Ok(Self {
            buffers: (0..BATCH_SIZE).map(|_| vec![0; full_len]).collect(),
            sizes: vec![0; BATCH_SIZE],
            len: 0,
            packet_capacity,
            #[cfg(target_os = "linux")]
            gso_scratch: vec![0; GSO_SCRATCH_LEN],
            #[cfg(target_os = "linux")]
            gro_table: GROTable::new(),
        })
    }

    /// The most bytes one packet may hold.
    pub fn packet_capacity(&self) -> usize {
        self.packet_capacity
    }

    /// Number of packets held.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the batch holds no packets.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the batch holds [`BATCH_SIZE`] packets.
    pub fn is_full(&self) -> bool {
        self.len == BATCH_SIZE
    }

    /// Forgets all packets. Buffers are kept for reuse.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Copies `packet` into the next free slot.
    pub fn push(&mut self, packet: &[u8]) -> Result<(), BatchError> {
        if packet.len() > self.packet_capacity {
            return Err(BatchError::PacketTooLarge {
                len: packet.len(),
                capacity: self.packet_capacity,
            });
        }
        let too_large = BatchError::PacketTooLarge {
            len: packet.len(),
            capacity: self.packet_capacity,
        };
        let slot = self.free_slot().ok_or(BatchError::Full)?;
        slot.get_mut(..packet.len())
            .ok_or(too_large)?
            .copy_from_slice(packet);
        self.commit(packet.len());
        Ok(())
    }

    /// The packet at `index`, if present.
    pub fn get(&self, index: usize) -> Option<&[u8]> {
        let size = *self.sizes.get(..self.len)?.get(index)?;
        self.buffers.get(index)?.get(HEADROOM..HEADROOM + size)
    }

    /// The packet at `index`, mutably, if present.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        let size = *self.sizes.get(..self.len)?.get(index)?;
        self.buffers
            .get_mut(index)?
            .get_mut(HEADROOM..HEADROOM + size)
    }

    /// The slot behind packet `index`, including its headroom.
    pub fn buffer_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        if index >= self.len {
            return None;
        }
        self.buffers.get_mut(index).map(Vec::as_mut_slice)
    }

    /// The packets in order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.buffers
            .iter()
            .zip(&self.sizes)
            .take(self.len)
            .filter_map(|(buffer, &size)| buffer.get(HEADROOM..HEADROOM + size))
    }

    /// The packet area of the next free slot, if any.
    pub(crate) fn free_slot(&mut self) -> Option<&mut [u8]> {
        self.buffers.get_mut(self.len)?.get_mut(HEADROOM..)
    }

    /// Records that the next free slot now holds `size` bytes.
    pub(crate) fn commit(&mut self, size: usize) {
        if let Some(slot_size) = self.sizes.get_mut(self.len) {
            *slot_size = size.min(self.packet_capacity);
            self.len += 1;
        }
    }

    /// Split borrows for `recv_multiple`.
    #[cfg(target_os = "linux")]
    pub(crate) fn gso_parts(&mut self) -> (&mut [u8], &mut [Vec<u8>], &mut [usize]) {
        (&mut self.gso_scratch, &mut self.buffers, &mut self.sizes)
    }

    /// Marks the first `count` slots as filled by `recv_multiple`.
    #[cfg(target_os = "linux")]
    pub(crate) fn set_len_after_gso(&mut self, count: usize) {
        self.len = count.min(BATCH_SIZE);
        let capacity = self.packet_capacity;
        for size in self.sizes.iter_mut().take(self.len) {
            *size = (*size).min(capacity);
        }
    }

    /// Shrinks used buffers to their packets, as tun-rs's GRO path expects.
    #[cfg(test)]
    pub(crate) fn prepare_send(&mut self) -> &mut [Vec<u8>] {
        self.shrink_to_packets();
        self.buffers.get_mut(..self.len).unwrap_or_default()
    }

    /// [`Self::prepare_send`] plus the GRO table.
    #[cfg(target_os = "linux")]
    pub(crate) fn prepare_gro_send(&mut self) -> (&mut GROTable, &mut [Vec<u8>]) {
        self.shrink_to_packets();
        let buffers = self.buffers.get_mut(..self.len).unwrap_or_default();
        (&mut self.gro_table, buffers)
    }

    #[cfg(any(target_os = "linux", test))]
    fn shrink_to_packets(&mut self) {
        for (buffer, &size) in self.buffers.iter_mut().zip(&self.sizes).take(self.len) {
            buffer.truncate(HEADROOM + size);
        }
    }

    /// Restores full buffer lengths and empties the batch.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn restore_after_send(&mut self) {
        let full_len = HEADROOM + self.packet_capacity;
        for buffer in &mut self.buffers {
            buffer.resize(full_len, 0);
        }
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Operation {
        Push(Vec<u8>),
        Clear,
        Overwrite { index: usize, byte: u8 },
        SendCycle { grow_first_by: usize },
    }

    fn operation(capacity: usize) -> impl Strategy<Value = Operation> {
        prop_oneof![
            8 => proptest::collection::vec(any::<u8>(), 0..=capacity + 4).prop_map(Operation::Push),
            1 => Just(Operation::Clear),
            2 => (0..BATCH_SIZE + 2, any::<u8>()).prop_map(|(index, byte)| Operation::Overwrite { index, byte }),
            1 => (0..=capacity).prop_map(|grow_first_by| Operation::SendCycle { grow_first_by }),
        ]
    }

    proptest! {
        #[test]
        fn batch_matches_model_and_never_reallocates(
            (capacity, operations) in (1..=96usize).prop_flat_map(|capacity| {
                (Just(capacity), proptest::collection::vec(operation(capacity), 0..400))
            })
        ) {
            let mut batch = PacketBatch::new(capacity).unwrap();
            let full_len = HEADROOM + capacity;
            let allocations: Vec<(*const u8, usize)> =
                batch.buffers.iter().map(|b| (b.as_ptr(), b.capacity())).collect();
            let mut model: Vec<Vec<u8>> = Vec::new();

            for operation in operations {
                match operation {
                    Operation::Push(packet) => {
                        let result = batch.push(&packet);
                        if packet.len() > capacity {
                            prop_assert_eq!(result, Err(BatchError::PacketTooLarge { len: packet.len(), capacity }));
                        } else if model.len() == BATCH_SIZE {
                            prop_assert_eq!(result, Err(BatchError::Full));
                        } else {
                            prop_assert_eq!(result, Ok(()));
                            model.push(packet);
                        }
                    }
                    Operation::Clear => {
                        batch.clear();
                        model.clear();
                    }
                    Operation::Overwrite { index, byte } => {
                        let headroom = batch.buffer_mut(index).map(|b| b.get_mut(..HEADROOM).map(|h| h.fill(byte)));
                        prop_assert_eq!(headroom.is_some(), index < model.len());
                        if let (Some(packet), Some(expected)) = (batch.get_mut(index), model.get_mut(index)) {
                            packet.fill(byte);
                            expected.fill(byte);
                        }
                    }
                    Operation::SendCycle { grow_first_by } => {
                        // Mimics tun-rs GRO growing a buffer within capacity.
                        let buffers = batch.prepare_send();
                        for (buffer, packet) in buffers.iter().zip(&model) {
                            prop_assert_eq!(buffer.len(), HEADROOM + packet.len());
                        }
                        if let Some(first) = buffers.first_mut() {
                            let target = (first.len() + grow_first_by).min(first.capacity());
                            first.resize(target, 0xAA);
                        }
                        batch.restore_after_send();
                        model.clear();
                    }
                }

                prop_assert_eq!(batch.len(), model.len());
                prop_assert!(batch.len() <= BATCH_SIZE);
                prop_assert_eq!(batch.is_full(), model.len() == BATCH_SIZE);
                prop_assert!(batch.iter().eq(model.iter().map(Vec::as_slice)));
                prop_assert_eq!(batch.get(model.len()), None);
                for buffer in &batch.buffers {
                    prop_assert_eq!(buffer.len(), full_len);
                }
                let now: Vec<(*const u8, usize)> =
                    batch.buffers.iter().map(|b| (b.as_ptr(), b.capacity())).collect();
                prop_assert_eq!(&now, &allocations);
            }
        }
    }
}
