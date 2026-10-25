//! Interface snapshots and change polling.
//!
//! Polling is portable; netlink and route-socket push events are a later optimisation.

use core::net::IpAddr;
use core::time::Duration;

use kith_crypto::{Context, Hash};
use tokio::sync::mpsc;

use crate::gateway::default_gateway;

/// Domain separation for [`NetworkSnapshot::fingerprint`].
const FINGERPRINT_CONTEXT: Context = Context::new("kith 2026-10 netmon fingerprint v1");

/// How often the monitor re-reads interfaces.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// IPv6 addresses are reduced to their /64 so privacy-address rotation is not a network change.
const IPV6_FINGERPRINT_PREFIX: u8 = 64;

/// One usable address on one interface.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceAddress {
    /// Interface name, such as `en0`.
    pub interface: String,
    /// The address.
    pub address: IpAddr,
    /// The prefix length of the on-link network.
    pub prefix_length: u8,
}

/// A sorted, deduplicated view of the host's usable addresses and default gateway.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NetworkSnapshot {
    addresses: Vec<InterfaceAddress>,
    gateway: Option<IpAddr>,
}

impl NetworkSnapshot {
    /// Builds a snapshot, sorting and deduplicating `addresses`.
    #[must_use]
    pub fn new(mut addresses: Vec<InterfaceAddress>, gateway: Option<IpAddr>) -> Self {
        addresses.sort();
        addresses.dedup();
        Self { addresses, gateway }
    }

    /// Reads the current interfaces, skipping loopback, link-local and the interface named `tun_name`.
    ///
    /// # Errors
    /// Returns the OS error when interfaces cannot be listed.
    pub fn capture(tun_name: &str) -> std::io::Result<Self> {
        let addresses = if_addrs::get_if_addrs()?
            .into_iter()
            .filter(|interface| {
                interface.name != tun_name && !interface.is_loopback() && !interface.is_link_local()
            })
            .map(|interface| {
                let prefix_length = match &interface.addr {
                    if_addrs::IfAddr::V4(v4) => v4.prefixlen,
                    if_addrs::IfAddr::V6(v6) => v6.prefixlen,
                };
                InterfaceAddress {
                    address: interface.ip(),
                    interface: interface.name,
                    prefix_length,
                }
            })
            .collect();
        Ok(Self::new(addresses, default_gateway()))
    }

    /// The addresses, sorted.
    #[must_use]
    pub fn addresses(&self) -> &[InterfaceAddress] {
        &self.addresses
    }

    /// The IPv4 default gateway, when known.
    #[must_use]
    pub const fn gateway(&self) -> Option<IpAddr> {
        self.gateway
    }

    /// A stable hash of the gateway, full IPv4 addresses and IPv6 /64 prefixes; interface names are excluded.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut keys: Vec<[u8; 17]> = self
            .addresses
            .iter()
            .map(|entry| key(entry.address))
            .collect();
        for key in &mut keys {
            if key.first() == Some(&6) {
                *key = mask_v6(*key);
            }
        }
        keys.sort_unstable();
        keys.dedup();
        let mut data = Vec::with_capacity(keys.len().saturating_add(1).saturating_mul(17));
        data.extend_from_slice(&self.gateway.map_or([0; 17], key));
        for key in keys {
            data.extend_from_slice(&key);
        }
        Hash::of(FINGERPRINT_CONTEXT, &data).to_bytes()
    }
}

fn key(address: IpAddr) -> [u8; 17] {
    let mut out = [0; 17];
    let (tag, rest) = out.split_at_mut(1);
    match address {
        IpAddr::V4(v4) => {
            tag.copy_from_slice(&[4]);
            if let Some(slot) = rest.get_mut(..4) {
                slot.copy_from_slice(&v4.octets());
            }
        }
        IpAddr::V6(v6) => {
            tag.copy_from_slice(&[6]);
            rest.copy_from_slice(&v6.octets());
        }
    }
    out
}

fn mask_v6(mut key: [u8; 17]) -> [u8; 17] {
    let keep = usize::from(IPV6_FINGERPRINT_PREFIX / 8);
    if let Some(host) = key.get_mut(keep.saturating_add(1)..) {
        host.fill(0);
    }
    key
}

/// The network changed between two polls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The previous snapshot.
    pub old: NetworkSnapshot,
    /// The current snapshot.
    pub new: NetworkSnapshot,
}

/// Spawns a task that polls every 5 s and sends a [`Change`] when the snapshot differs.
///
/// The task ends when the receiver is dropped.
#[must_use]
pub fn spawn_monitor(tun_name: String) -> mpsc::Receiver<Change> {
    let (sender, receiver) = mpsc::channel(8);
    tokio::spawn(async move {
        let capture = |name: String| async move {
            tokio::task::spawn_blocking(move || NetworkSnapshot::capture(&name).ok())
                .await
                .ok()
                .flatten()
        };
        let mut current = capture(tun_name.clone()).await;
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = sender.closed() => return,
                _ = interval.tick() => {}
            }
            let Some(new) = capture(tun_name.clone()).await else {
                continue;
            };
            match current.replace(new.clone()) {
                Some(old) if old != new => {
                    if sender.send(Change { old, new }).await.is_err() {
                        return;
                    }
                }
                _ => {}
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::net::{Ipv4Addr, Ipv6Addr};

    fn entry(interface: &str, address: IpAddr, prefix_length: u8) -> InterfaceAddress {
        InterfaceAddress {
            interface: interface.to_owned(),
            address,
            prefix_length,
        }
    }

    #[test]
    fn fingerprint_ignores_order_and_v6_host_bits_but_tracks_addresses() {
        let wifi = entry("en0", IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 24);
        let v6 = |host: u16| {
            entry(
                "en0",
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, host)),
                64,
            )
        };
        let lte = entry("pdp_ip0", IpAddr::V4(Ipv4Addr::new(10, 64, 0, 3)), 32);
        let gateway = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));

        let base = NetworkSnapshot::new(vec![wifi.clone(), v6(1), lte.clone()], gateway);
        let shuffled = NetworkSnapshot::new(vec![lte.clone(), v6(1), wifi.clone()], gateway);
        assert_eq!(base, shuffled);
        assert_eq!(base.fingerprint(), shuffled.fingerprint());

        let rotated = NetworkSnapshot::new(vec![wifi.clone(), v6(2), lte.clone()], gateway);
        assert_eq!(base.fingerprint(), rotated.fingerprint());

        let moved = entry("en0", IpAddr::V4(Ipv4Addr::new(192, 168, 1, 21)), 24);
        let readdressed = NetworkSnapshot::new(vec![moved, v6(1), lte.clone()], gateway);
        assert_ne!(base.fingerprint(), readdressed.fingerprint());

        let regatewayed = NetworkSnapshot::new(vec![wifi, v6(1), lte], None);
        assert_ne!(base.fingerprint(), regatewayed.fingerprint());
    }
}
