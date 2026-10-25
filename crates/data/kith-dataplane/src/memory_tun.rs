//! An in-memory TUN for tests that run without root.

use kith_tun::{BatchError, MAX_MTU, PacketBatch, TunError};
use tokio::sync::{Mutex, mpsc};

use crate::api::DataplaneError;
use crate::tun_device::TunDevice;

/// The test's end of an in-memory TUN.
#[derive(Debug)]
pub struct MemoryTun {
    inject: mpsc::UnboundedSender<Vec<u8>>,
    delivered: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl MemoryTun {
    /// Queues a packet as if the OS had routed it into the TUN.
    pub fn inject(&self, packet: Vec<u8>) -> Result<(), DataplaneError> {
        self.inject
            .send(packet)
            .map_err(|_| DataplaneError::Stopped)
    }

    /// The next packet the dataplane wrote to the TUN, or `None` once it stopped.
    pub async fn receive(&mut self) -> Option<Vec<u8>> {
        self.delivered.recv().await
    }

    /// A packet already written to the TUN, without waiting.
    pub fn try_receive(&mut self) -> Option<Vec<u8>> {
        self.delivered.try_recv().ok()
    }
}

pub(crate) struct MemoryDevice {
    incoming: Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
}

pub(crate) fn memory_tun() -> (MemoryDevice, MemoryTun) {
    let (inject, incoming) = mpsc::unbounded_channel();
    let (outgoing, delivered) = mpsc::unbounded_channel();
    (
        MemoryDevice {
            incoming: Mutex::new(incoming),
            outgoing,
        },
        MemoryTun { inject, delivered },
    )
}

impl TunDevice for MemoryDevice {
    fn new_batch(&self) -> Result<PacketBatch, BatchError> {
        PacketBatch::new(usize::from(MAX_MTU))
    }

    async fn recv_batch(&self, batch: &mut PacketBatch) -> Result<usize, TunError> {
        batch.clear();
        let mut incoming = self.incoming.lock().await;
        let Some(first) = incoming.recv().await else {
            return core::future::pending().await;
        };
        batch.push(&first).ok();
        while !batch.is_full() {
            let Ok(packet) = incoming.try_recv() else {
                break;
            };
            batch.push(&packet).ok();
        }
        Ok(batch.len())
    }

    fn send_batch(
        &self,
        batch: &mut PacketBatch,
    ) -> impl Future<Output = Result<(), TunError>> + Send {
        for packet in batch.iter() {
            self.outgoing.send(packet.to_vec()).ok();
        }
        batch.clear();
        core::future::ready(Ok(()))
    }
}
