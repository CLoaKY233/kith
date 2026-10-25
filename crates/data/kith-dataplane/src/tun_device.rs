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

pub(crate) enum AnyTun {
    Os(Tun),
    #[cfg(feature = "test-support")]
    Memory(crate::memory_tun::MemoryDevice),
}

impl TunDevice for AnyTun {
    fn new_batch(&self) -> Result<PacketBatch, BatchError> {
        match self {
            Self::Os(tun) => tun.new_batch(),
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => tun.new_batch(),
        }
    }

    async fn recv_batch(&self, batch: &mut PacketBatch) -> Result<usize, TunError> {
        match self {
            Self::Os(tun) => tun.recv_batch(batch).await,
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => TunDevice::recv_batch(tun, batch).await,
        }
    }

    async fn send_batch(&self, batch: &mut PacketBatch) -> Result<(), TunError> {
        match self {
            Self::Os(tun) => tun.send_batch(batch).await,
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => TunDevice::send_batch(tun, batch).await,
        }
    }
}
