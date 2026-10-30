use core::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use kith_control::records::{DnsData, DnsRecords};
use kith_control::{NetMap, Networks};
use kith_dns::{DnsServer, DnsState};
use kith_naming::{DomainName, Record, RecordData, Txt, Zone, ZoneBuilder};
use kith_sys::Domain;
use tokio::task::JoinHandle;

use crate::types::NodeError;

/// The DNS server answering for every joined network on the resolver address.
#[derive(Debug)]
pub(crate) struct DnsService {
    state: Arc<DnsState>,
    local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl DnsService {
    /// Binds the server on `listen`, forwarding other names to `upstreams`.
    pub(crate) async fn start(
        listen: SocketAddr,
        upstreams: Vec<SocketAddr>,
    ) -> Result<Self, NodeError> {
        let state = Arc::new(DnsState::new(Zone::empty()));
        state.set_default_upstreams(upstreams);
        let server = DnsServer::bind(listen, Arc::clone(&state)).await?;
        let local_addr = server.local_addr()?;
        let task = tokio::spawn(server.run());
        Ok(Self {
            state,
            local_addr,
            task,
        })
    }

    /// Replaces where names outside the mesh are forwarded.
    pub(crate) fn set_upstreams(&self, upstreams: Vec<SocketAddr>) {
        self.state.set_default_upstreams(upstreams);
    }

    /// The bound address.
    pub(crate) const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Rebuilds the zone and split-DNS routes from every network map.
    pub(crate) fn update(&self, maps: &Networks) -> Result<(), NodeError> {
        self.state.set_zone(build_zone(maps)?);
        Ok(())
    }

    /// The domains the OS must send to this server.
    #[expect(clippy::unused_self, reason = "kept as a method for the daemon")]
    pub(crate) fn domains(&self, maps: &Networks) -> Vec<Domain> {
        maps.iter()
            .filter_map(|(_, map)| suffix_of(map))
            .filter_map(|suffix| Domain::new(suffix.as_str()).ok())
            .collect()
    }
}

impl Drop for DnsService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Every DNS name of `map`: its devices' first, then the custom records, each sorted.
pub(crate) fn record_list(map: &NetMap) -> Vec<kith_ipc::RecordInfo> {
    use kith_ipc::{DnsRecord, RecordInfo};
    let Some(suffix) = suffix_of(map) else {
        return Vec::new();
    };
    let clients = map.clients().values().map(|client| &client.peer);
    let mut devices: Vec<RecordInfo> = map
        .peers()
        .values()
        .chain(map.local_device())
        .chain(clients)
        .flat_map(|peer| {
            let name = format!("{}.{}", peer.label, suffix.as_str());
            [
                DnsRecord::A(peer.addresses.v4),
                DnsRecord::Aaaa(peer.addresses.v6),
            ]
            .map(|record| RecordInfo {
                name: name.clone(),
                record,
                device: true,
            })
        })
        .collect();
    let mut custom: Vec<RecordInfo> = map
        .dns_records()
        .iter()
        .flat_map(|set| {
            let name = if set.name() == "@" {
                suffix.as_str().to_owned()
            } else {
                format!("{}.{}", set.name(), suffix.as_str())
            };
            set.data().iter().map(move |data| RecordInfo {
                name: name.clone(),
                record: match data {
                    DnsData::A(v4) => DnsRecord::A(*v4),
                    DnsData::Aaaa(v6) => DnsRecord::Aaaa(*v6),
                    DnsData::Cname(target) => DnsRecord::Cname(target.clone()),
                    DnsData::Txt(parts) => DnsRecord::Txt(parts.concat()),
                },
                device: false,
            })
        })
        .collect();
    let order = |a: &RecordInfo, b: &RecordInfo| a.name.cmp(&b.name);
    devices.sort_by(order);
    custom.sort_by(order);
    devices.extend(custom);
    devices
}

/// Builds one zone from device names and `dns/` records of every network.
pub(crate) fn build_zone(maps: &Networks) -> Result<Zone, NodeError> {
    let mut builder = ZoneBuilder::new();
    let mut any = false;
    for (_, map) in maps.iter() {
        let Some(suffix) = suffix_of(map) else {
            continue;
        };
        let devices = device_records(map, &suffix);
        let custom = custom_records(map, &suffix);
        let mut trial = ZoneBuilder::new();
        trial.add_suffix(suffix.clone())?;
        add_all(&mut trial, devices.iter().chain(&custom))?;
        let entries: Vec<&(DomainName, Record)> = match trial.build() {
            Ok(_) => devices.iter().chain(&custom).collect(),
            Err(error) => {
                tracing::warn!(network = map.name(), %error, "ignoring custom DNS records");
                devices.iter().collect()
            }
        };
        builder.add_suffix(suffix)?;
        add_all(&mut builder, entries)?;
        any = true;
    }
    if any {
        Ok(builder.build()?)
    } else {
        Ok(Zone::empty())
    }
}

fn add_all<'a>(
    builder: &mut ZoneBuilder,
    entries: impl IntoIterator<Item = &'a (DomainName, Record)>,
) -> Result<(), NodeError> {
    for (name, record) in entries {
        builder.add_record(name.clone(), record.clone())?;
    }
    Ok(())
}

fn suffix_of(map: &NetMap) -> Option<DomainName> {
    if !map.is_member() {
        return None;
    }
    let suffix = DomainName::new(&format!("{}.kith.internal", map.name()));
    if suffix.is_err() {
        tracing::warn!(network = map.name(), "network name is not a DNS label");
    }
    suffix.ok()
}

fn device_records(map: &NetMap, suffix: &DomainName) -> Vec<(DomainName, Record)> {
    let mut records = Vec::new();
    let clients = map.clients().values().map(|client| &client.peer);
    for peer in map
        .peers()
        .values()
        .chain(map.local_device())
        .chain(clients)
    {
        let Ok(name) = suffix.child(&peer.label) else {
            tracing::warn!(device = peer.label, "device name is not a DNS label");
            continue;
        };
        for address in [IpAddr::V4(peer.addresses.v4), IpAddr::V6(peer.addresses.v6)] {
            let data = match address {
                IpAddr::V4(v4) => RecordData::A(v4),
                IpAddr::V6(v6) => RecordData::Aaaa(v6),
            };
            if let Ok(record) = Record::new(DEVICE_TTL, data) {
                records.push((name.clone(), record));
            }
        }
    }
    records
}

const DEVICE_TTL: u32 = 60;

fn custom_records(map: &NetMap, suffix: &DomainName) -> Vec<(DomainName, Record)> {
    let mut records = Vec::new();
    for set in map.dns_records() {
        match custom_set(set, suffix) {
            Ok(set) => records.extend(set),
            Err(error) => {
                tracing::warn!(network = map.name(), name = set.name(), %error, "invalid DNS record");
            }
        }
    }
    records
}

fn custom_set(
    set: &DnsRecords,
    suffix: &DomainName,
) -> Result<Vec<(DomainName, Record)>, CustomError> {
    let name = if set.name() == "@" {
        suffix.clone()
    } else {
        DomainName::new(&format!("{}.{}", set.name(), suffix.as_str()))?
    };
    set.data()
        .iter()
        .map(|data| {
            let data = match data {
                DnsData::A(v4) => RecordData::A(*v4),
                DnsData::Aaaa(v6) => RecordData::Aaaa(*v6),
                DnsData::Cname(target) => RecordData::Cname(DomainName::new(target)?),
                DnsData::Txt(strings) => RecordData::Txt(Txt::new(strings.clone())?),
            };
            Ok((name.clone(), Record::new(set.ttl(), data)?))
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
enum CustomError {
    #[error(transparent)]
    Name(#[from] kith_naming::NameError),
    #[error(transparent)]
    Record(#[from] kith_naming::RecordError),
}
