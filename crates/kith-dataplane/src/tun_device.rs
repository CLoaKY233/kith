//! The TUN operations the engine uses, so tests can swap in memory queues.

use kith_tun::{BatchError, PacketBatch, Tun, TunError};

pub(crate) trait TunDevice {
    fn new_batch(&self) -> Result<PacketBatch, BatchError>;

    fn recv_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<usize, TunError>> + Send;

    fn send_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<(), TunError>> + Send;
}

impl TunDevice for Tun {
    fn new_batch(&self) -> Result<PacketBatch, BatchError> {
        Self::new_batch(self)
    }

    fn recv_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<usize, TunError>> + Send {
        Self::recv_batch(self, batch)
    }

    fn send_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<(), TunError>> + Send {
        Self::send_batch(self, batch)
    }
}
