//! The TUN operations the engine uses, so tests can swap in memory queues.

use std::net::IpAddr;

use kith_tun::{BatchError, PacketBatch, Tun, TunError};

pub(crate) trait TunDevice {
    /// The largest packet the device carries.
    fn mtu(&self) -> u16;

    fn new_batch(&self) -> Result<PacketBatch, BatchError>;

    fn recv_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<usize, TunError>> + Send;

    fn send_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<(), TunError>> + Send;

    fn add_address(&self, address: IpAddr, prefix_len: u8) -> Result<(), TunError>;

    fn remove_address(&self, address: IpAddr) -> Result<(), TunError>;
}

impl TunDevice for Tun {
    fn mtu(&self) -> u16 {
        Self::mtu(self)
    }

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

    fn add_address(&self, address: IpAddr, prefix_len: u8) -> Result<(), TunError> {
        Self::add_address(self, address, prefix_len)
    }

    fn remove_address(&self, address: IpAddr) -> Result<(), TunError> {
        Self::remove_address(self, address)
    }
}

pub(crate) enum AnyTun {
    Os(Tun),
    #[cfg(feature = "test-support")]
    Memory(crate::memory_tun::MemoryDevice),
}

impl TunDevice for AnyTun {
    fn mtu(&self) -> u16 {
        match self {
            Self::Os(tun) => tun.mtu(),
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => tun.mtu(),
        }
    }

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

    fn add_address(&self, address: IpAddr, prefix_len: u8) -> Result<(), TunError> {
        match self {
            Self::Os(tun) => tun.add_address(address, prefix_len),
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => tun.add_address(address, prefix_len),
        }
    }

    fn remove_address(&self, address: IpAddr) -> Result<(), TunError> {
        match self {
            Self::Os(tun) => tun.remove_address(address),
            #[cfg(feature = "test-support")]
            Self::Memory(tun) => tun.remove_address(address),
        }
    }
}
