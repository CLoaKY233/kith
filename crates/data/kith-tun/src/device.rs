//! The TUN device and its batched read and write paths.

use std::io;
use std::net::IpAddr;

use tracing::debug;
use tun_rs::AsyncDevice;

use crate::batch::{BatchError, PacketBatch};
use crate::config::{ConfigError, TunConfig, validate_address};

/// Why creating or using a [`Tun`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TunError {
    /// The configuration was rejected before touching the OS.
    #[error("invalid TUN configuration: {0}")]
    InvalidConfig(#[from] ConfigError),
    /// The OS refused to create or configure the device.
    #[error(
        "permission denied creating or configuring the TUN device: run as root, or grant CAP_NET_ADMIN on Linux \
         (for example `sudo setcap cap_net_admin+ep <binary>`)"
    )]
    PermissionDenied(#[source] io::Error),
    /// This platform or build cannot do what was asked.
    #[error("not supported on this platform: {0}")]
    Unsupported(&'static str),
    /// [`Tun::create`] was called outside a Tokio runtime.
    #[error("Tun::create must be called from within a Tokio runtime")]
    NoRuntime,
    /// Batch slots are smaller than the MTU.
    #[error("batch packet capacity {capacity} is below the device MTU {mtu}")]
    BatchTooSmall {
        /// The batch's per-packet capacity.
        capacity: usize,
        /// The device MTU.
        mtu: u16,
    },
    /// Any other OS error.
    #[error("TUN device I/O failed: {0}")]
    Io(#[source] io::Error),
}

impl From<io::Error> for TunError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::PermissionDenied {
            Self::PermissionDenied(error)
        } else {
            Self::Io(error)
        }
    }
}

/// An open TUN device. Dropping it removes the interface.
pub struct Tun {
    device: AsyncDevice,
    name: String,
    mtu: u16,
    offload: bool,
}

impl core::fmt::Debug for Tun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Tun")
            .field("name", &self.name)
            .field("mtu", &self.mtu)
            .field("offload", &self.offload)
            .finish_non_exhaustive()
    }
}

impl Tun {
    /// Creates the interface, assigns its addresses and brings it up.
    /// Must run inside a Tokio runtime.
    pub fn create(config: &TunConfig) -> Result<Self, TunError> {
        config.validate()?;
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(TunError::NoRuntime);
        }
        let (device, offload) = open(config)?;
        let name = device.name()?;
        Ok(Self {
            device,
            name,
            mtu: config.mtu,
            offload,
        })
    }

    /// The interface name chosen at creation, such as `utun7` or `kith0`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The interface MTU.
    pub fn mtu(&self) -> u16 {
        self.mtu
    }

    /// Whether reads and writes go through GSO/GRO offload.
    pub fn offload_enabled(&self) -> bool {
        self.offload
    }

    /// Adds `address/prefix_len` beside the existing addresses. An address the
    /// device already has, under any prefix length, is left as is. Returns
    /// [`TunError::Unsupported`] except on Linux, macOS and the BSDs.
    pub fn add_address(&self, address: IpAddr, prefix_len: u8) -> Result<(), TunError> {
        validate_address(address, prefix_len, self.mtu)?;
        if crate::address::add(&self.device, address, prefix_len)? {
            debug!(device = %self.name, %address, prefix_len, "added TUN address");
        }
        Ok(())
    }

    /// Removes `address` whatever its prefix length; an address the device does
    /// not have is not an error. Returns [`TunError::Unsupported`] except on
    /// Linux, macOS and the BSDs.
    pub fn remove_address(&self, address: IpAddr) -> Result<(), TunError> {
        if crate::address::remove(&self.device, address)? {
            debug!(device = %self.name, %address, "removed TUN address");
        }
        Ok(())
    }

    /// A batch sized for this device's MTU.
    pub fn new_batch(&self) -> Result<PacketBatch, BatchError> {
        PacketBatch::new(usize::from(self.mtu))
    }

    /// Refills `batch` with at least one packet and returns the count.
    /// Splits GSO superpackets when offload is on.
    pub async fn recv_batch(&self, batch: &mut PacketBatch) -> Result<usize, TunError> {
        if batch.packet_capacity() < usize::from(self.mtu) {
            return Err(TunError::BatchTooSmall {
                capacity: batch.packet_capacity(),
                mtu: self.mtu,
            });
        }
        batch.clear();
        #[cfg(target_os = "linux")]
        if self.offload {
            let (scratch, buffers, sizes) = batch.gso_parts();
            let count = self
                .device
                .recv_multiple(scratch, buffers, sizes, crate::batch::HEADROOM)
                .await?;
            batch.set_len_after_gso(count);
            return Ok(batch.len());
        }
        self.recv_plain(batch).await
    }

    async fn recv_plain(&self, batch: &mut PacketBatch) -> Result<usize, TunError> {
        let Some(slot) = batch.free_slot() else {
            return Ok(0);
        };
        let size = self.device.recv(slot).await?;
        batch.commit(size);
        while let Some(slot) = batch.free_slot() {
            match self.device.try_recv(slot) {
                Ok(size) => batch.commit(size),
                Err(_would_block_or_error) => break,
            }
        }
        Ok(batch.len())
    }

    /// Writes every packet, GRO-coalescing when offload is on, then clears
    /// `batch` (even on error, since GRO rewrites its buffers).
    pub async fn send_batch(&self, batch: &mut PacketBatch) -> Result<(), TunError> {
        #[cfg(target_os = "linux")]
        if self.offload {
            let (gro_table, buffers) = batch.prepare_gro_send();
            let result = self
                .device
                .send_multiple(gro_table, buffers, crate::batch::HEADROOM)
                .await;
            batch.restore_after_send();
            return result.map(drop).map_err(TunError::from);
        }
        let result = self.send_plain(batch).await;
        batch.clear();
        result
    }

    async fn send_plain(&self, batch: &PacketBatch) -> Result<(), TunError> {
        for packet in batch.iter() {
            self.device.send(packet).await?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn builder(config: &TunConfig) -> tun_rs::DeviceBuilder {
    let mut builder = tun_rs::DeviceBuilder::new().mtu(config.mtu);
    if let Some(name) = &config.name {
        builder = builder.name(name.as_str());
    }
    if let Some((address, prefix)) = config.ipv4 {
        builder = builder.ipv4(address, prefix, None);
    }
    if let Some((address, prefix)) = config.ipv6 {
        builder = builder.ipv6(address, prefix);
    }
    builder
}

/// Opens the device, returning it and whether offload is on.
#[cfg(target_os = "linux")]
fn open(config: &TunConfig) -> Result<(AsyncDevice, bool), TunError> {
    if !config.offload {
        return Ok((builder(config).build_async()?, false));
    }
    let refusal = match builder(config).offload(true).build_async() {
        Ok(device) => {
            if !device.udp_gso() {
                debug!("TUN UDP segmentation offload unavailable (needs Linux 6.2)");
            }
            let offload = device.tcp_gso();
            return Ok((device, offload));
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return Err(error.into());
        }
        Err(error) => error,
    };
    let device = builder(config).build_async()?;
    tracing::warn!(error = %refusal, "TUN offload refused, continuing without GSO/GRO");
    Ok((device, false))
}

/// Opens the device, returning it and whether offload is on (never, here).
#[cfg(all(unix, not(target_os = "linux")))]
fn open(config: &TunConfig) -> Result<(AsyncDevice, bool), TunError> {
    Ok((builder(config).build_async()?, false))
}

/// Windows needs `wintun.dll` beside the binary; kith does not ship it yet.
#[cfg(windows)]
fn open(_config: &TunConfig) -> Result<(AsyncDevice, bool), TunError> {
    Err(TunError::Unsupported(
        "Windows needs wintun.dll, which kith does not package yet",
    ))
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
    use std::time::Duration;

    use super::*;

    /// Destination of a UDP packet, if it is one.
    fn udp_destination(packet: &[u8]) -> Option<IpAddr> {
        match packet.first()? >> 4 {
            4 if *packet.get(9)? == 17 => {
                let destination: [u8; 4] = packet.get(16..20)?.try_into().ok()?;
                Some(Ipv4Addr::from(destination).into())
            }
            6 if *packet.get(6)? == 17 => {
                let destination: [u8; 16] = packet.get(24..40)?.try_into().ok()?;
                Some(Ipv6Addr::from(destination).into())
            }
            _ => None,
        }
    }

    /// Whether UDP sent to `target` comes out of `tun` within five seconds.
    async fn udp_arrives(tun: &Tun, target: SocketAddr) -> bool {
        let local: IpAddr = if target.is_ipv4() {
            Ipv4Addr::UNSPECIFIED.into()
        } else {
            Ipv6Addr::UNSPECIFIED.into()
        };
        let socket = UdpSocket::bind((local, 0)).unwrap();
        let mut batch = tun.new_batch().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                socket.send_to(b"kith", target).unwrap();
                tun.recv_batch(&mut batch).await.unwrap();
                if batch
                    .iter()
                    .any(|packet| udp_destination(packet) == Some(target.ip()))
                {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }

    /// Needs root: `sudo -E cargo nextest run -p kith-tun --run-ignored only`.
    #[tokio::test]
    #[ignore = "creates a real TUN device; needs root or CAP_NET_ADMIN"]
    async fn initial_and_added_subnets_reach_recv_batch_and_removal_keeps_the_rest() {
        let initial = Ipv4Addr::new(10, 213, 78, 1);
        let tun = Tun::create(&TunConfig {
            name: None,
            mtu: 1420,
            ipv4: Some((initial, 24)),
            ipv6: None,
            offload: true,
        })
        .unwrap();
        let initial_target = SocketAddr::from((Ipv4Addr::new(10, 213, 78, 2), 40_000));
        assert!(
            udp_arrives(&tun, initial_target).await,
            "no UDP packet to {initial_target} arrived on {}",
            tun.name()
        );
        let ipv6 = |host| Ipv6Addr::new(0xfd6b, 0x6974, 0x6800, 1, 0, 0, 0, host);
        let added: [(IpAddr, u8, SocketAddr); 2] = [
            (
                Ipv4Addr::new(10, 213, 79, 1).into(),
                24,
                (Ipv4Addr::new(10, 213, 79, 2), 40_000).into(),
            ),
            (ipv6(1).into(), 64, (ipv6(2), 40_000).into()),
        ];
        for &(address, prefix_len, target) in &added {
            tun.add_address(address, prefix_len).unwrap();
            tun.add_address(address, prefix_len).unwrap();
            assert!(
                udp_arrives(&tun, target).await,
                "no UDP packet to {target} arrived after adding {address}"
            );
        }
        for &(address, _, _) in &added {
            tun.remove_address(address).unwrap();
            tun.remove_address(address).unwrap();
        }
        let remaining = tun.device.addresses().unwrap();
        assert!(remaining.contains(&initial.into()), "{remaining:?}");
        for (address, _, _) in added {
            assert!(!remaining.contains(&address), "{remaining:?}");
        }
    }
}
