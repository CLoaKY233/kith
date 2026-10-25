//! The authoritative zone: device names, custom records and reverse pointers.
//!
//! A [`ZoneBuilder`] collects inputs in any order and [`ZoneBuilder::build`] validates
//! them all at once into an immutable [`Zone`]. The zone is an `Arc` inside, so the
//! driver can swap in a new zone on every netmap change and in-flight answers keep the
//! old one alive. All maps are `BTree`s, so the same inputs give the same zone on every
//! device, whatever order they arrived in.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::net::IpAddr;

use crate::name::{DomainName, NameError, is_within, self_and_ancestors};
use crate::record::{MAX_TTL, Record, RecordData, RecordType};

/// Most authoritative suffixes in one zone.
pub const MAX_SUFFIXES: usize = 16;
/// Most devices in one zone.
pub const MAX_DEVICES: usize = 4096;
/// Most addresses per device.
pub const MAX_ADDRESSES_PER_DEVICE: usize = 16;
/// Most custom records in one zone.
pub const MAX_CUSTOM_RECORDS: usize = 8192;
/// Most records at one name, after merging device and custom records.
pub const MAX_RECORDS_PER_NAME: usize = 64;
/// TTL of device and reverse records unless set otherwise.
pub const DEFAULT_DEVICE_TTL: u32 = 60;
/// TTL of negative answers (the SOA minimum, RFC 2308) unless set otherwise.
pub const DEFAULT_NEGATIVE_TTL: u32 = 30;

/// What happens when a custom record is placed at a device's name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OverridePolicy {
    /// The zone fails to build with [`ZoneError::ConflictsWithDevice`]. This stops a
    /// stale custom record from silently hijacking a device name.
    #[default]
    Reject,
    /// The custom records replace *all* of the device's records at that name. The
    /// device's reverse (PTR) records are unaffected.
    CustomWins,
}

/// Why a zone failed to build.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ZoneError {
    /// A zone needs at least one authoritative suffix.
    #[error("zone has no authoritative suffix")]
    NoSuffix,
    /// More than [`MAX_SUFFIXES`] suffixes.
    #[error("more than {} suffixes", MAX_SUFFIXES)]
    TooManySuffixes,
    /// The suffix was added twice.
    #[error("duplicate suffix {0}")]
    DuplicateSuffix(DomainName),
    /// Suffixes cannot be wildcards.
    #[error("suffix {0} is a wildcard")]
    WildcardSuffix(DomainName),
    /// A device name is not a single hostname label (letters, digits, inner `-`).
    #[error("invalid device name {0:?}")]
    InvalidDeviceName(String),
    /// `<device>.<suffix>` is not a valid name (normally: too long).
    #[error("device name under a suffix is invalid: {0}")]
    DeviceNameTooLong(NameError),
    /// The device was added twice.
    #[error("duplicate device {0}")]
    DuplicateDevice(DomainName),
    /// More than [`MAX_DEVICES`] devices.
    #[error("more than {} devices", MAX_DEVICES)]
    TooManyDevices,
    /// More than [`MAX_ADDRESSES_PER_DEVICE`] addresses on a device.
    #[error("device {0} has more than {max} addresses", max = MAX_ADDRESSES_PER_DEVICE)]
    TooManyAddresses(DomainName),
    /// Two devices claim the same address, so its PTR would be ambiguous.
    #[error("address {0} belongs to more than one device")]
    DuplicateAddress(IpAddr),
    /// More than [`MAX_CUSTOM_RECORDS`] custom records.
    #[error("more than {} custom records", MAX_CUSTOM_RECORDS)]
    TooManyRecords,
    /// A custom record lies outside every authoritative suffix.
    #[error("{0} is outside every authoritative suffix")]
    OutsideZone(DomainName),
    /// A custom record sits at a device name and the policy is [`OverridePolicy::Reject`].
    #[error("custom record at {0} conflicts with a device")]
    ConflictsWithDevice(DomainName),
    /// A CNAME shares its name with another record (RFC 1034 §3.6.2).
    #[error("CNAME at {0} is not the only record there")]
    CnameNotExclusive(DomainName),
    /// A CNAME at a suffix apex, which must keep its SOA.
    #[error("CNAME at zone apex {0}")]
    CnameAtApex(DomainName),
    /// More than [`MAX_RECORDS_PER_NAME`] records at one name.
    #[error("more than {max} records at {0}", max = MAX_RECORDS_PER_NAME)]
    TooManyRecordsAtName(DomainName),
    /// A TTL setting above [`MAX_TTL`].
    #[error("TTL {0} exceeds {max}", max = MAX_TTL)]
    TtlTooLarge(u32),
}

/// Collects zone inputs; see [`ZoneBuilder::build`] for the validation rules.
#[derive(Debug, Clone, Default)]
pub struct ZoneBuilder {
    suffixes: Vec<DomainName>,
    devices: BTreeMap<DomainName, Vec<IpAddr>>,
    custom: Vec<(DomainName, Record)>,
    policy: OverridePolicy,
    device_ttl: Option<u32>,
    negative_ttl: Option<u32>,
    serial: u32,
}

impl ZoneBuilder {
    /// An empty builder with default TTLs, serial 0 and [`OverridePolicy::Reject`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an authoritative suffix. The first one is the *primary* suffix, which reverse
    /// (PTR) records point into.
    pub fn add_suffix(&mut self, suffix: DomainName) -> Result<&mut Self, ZoneError> {
        if suffix.is_wildcard() {
            return Err(ZoneError::WildcardSuffix(suffix));
        }
        if self.suffixes.contains(&suffix) {
            return Err(ZoneError::DuplicateSuffix(suffix));
        }
        if self.suffixes.len() >= MAX_SUFFIXES {
            return Err(ZoneError::TooManySuffixes);
        }
        self.suffixes.push(suffix);
        Ok(self)
    }

    /// Adds a device, published as `<name>.<suffix>` under every suffix with one A or
    /// AAAA record per address. `name` must be a single hostname label.
    pub fn add_device(&mut self, name: &str, addresses: &[IpAddr]) -> Result<&mut Self, ZoneError> {
        let label = DomainName::new(name)
            .ok()
            .filter(|label| !label.as_str().contains(['.', '_', '*']))
            .ok_or_else(|| ZoneError::InvalidDeviceName(String::from(name)))?;
        if self.devices.contains_key(&label) {
            return Err(ZoneError::DuplicateDevice(label));
        }
        if self.devices.len() >= MAX_DEVICES {
            return Err(ZoneError::TooManyDevices);
        }
        if addresses.len() > MAX_ADDRESSES_PER_DEVICE {
            return Err(ZoneError::TooManyAddresses(label));
        }
        self.devices.insert(label, addresses.to_vec());
        Ok(self)
    }

    /// Adds a custom record at an absolute `name`, which must lie within a suffix.
    pub fn add_record(&mut self, name: DomainName, record: Record) -> Result<&mut Self, ZoneError> {
        if self.custom.len() >= MAX_CUSTOM_RECORDS {
            return Err(ZoneError::TooManyRecords);
        }
        self.custom.push((name, record));
        Ok(self)
    }

    /// Sets the device/custom conflict rule.
    pub fn override_policy(&mut self, policy: OverridePolicy) -> &mut Self {
        self.policy = policy;
        self
    }

    /// Sets the TTL of device A/AAAA and PTR records.
    pub fn device_ttl(&mut self, ttl: u32) -> &mut Self {
        self.device_ttl = Some(ttl);
        self
    }

    /// Sets the negative-caching TTL, used as the SOA TTL and minimum.
    pub fn negative_ttl(&mut self, ttl: u32) -> &mut Self {
        self.negative_ttl = Some(ttl);
        self
    }

    /// Sets the SOA serial. The caller derives it from replicated state (for example
    /// the ledger version) so every device serves the same serial.
    pub fn serial(&mut self, serial: u32) -> &mut Self {
        self.serial = serial;
        self
    }

    /// Validates every input and builds the zone.
    ///
    /// Rules, checked in this order:
    /// 1. at least one suffix;
    /// 2. every custom record lies within a suffix;
    /// 3. a custom record at a device name obeys the [`OverridePolicy`];
    /// 4. identical records are merged, and each `RRset` takes the lowest TTL in it
    ///    (RFC 2181 §5.2);
    /// 5. a CNAME is the only record at its name and never sits at an apex;
    /// 6. at most [`MAX_RECORDS_PER_NAME`] records per name;
    /// 7. no address belongs to two devices.
    pub fn build(&self) -> Result<Zone, ZoneError> {
        let primary = self.suffixes.first().ok_or(ZoneError::NoSuffix)?;
        let device_ttl = checked_ttl(self.device_ttl.unwrap_or(DEFAULT_DEVICE_TTL))?;
        let negative_ttl = checked_ttl(self.negative_ttl.unwrap_or(DEFAULT_NEGATIVE_TTL))?;

        let mut nodes: BTreeMap<DomainName, Vec<Record>> = BTreeMap::new();
        for (label, addresses) in &self.devices {
            for suffix in &self.suffixes {
                let name = suffix
                    .child(label.as_str())
                    .map_err(ZoneError::DeviceNameTooLong)?;
                let records = nodes.entry(name).or_default();
                for address in addresses {
                    let data = match *address {
                        IpAddr::V4(v4) => RecordData::A(v4),
                        IpAddr::V6(v6) => RecordData::Aaaa(v6),
                    };
                    records.push(
                        Record::new(device_ttl, data)
                            .map_err(|_| ZoneError::TtlTooLarge(device_ttl))?,
                    );
                }
            }
        }

        let mut overridden = BTreeSet::new();
        for (name, record) in &self.custom {
            if !self.suffixes.iter().any(|suffix| name.is_within(suffix)) {
                return Err(ZoneError::OutsideZone(name.clone()));
            }
            let records = nodes.entry(name.clone()).or_default();
            let is_device = !overridden.contains(name) && self.is_device_name(name);
            if is_device {
                match self.policy {
                    OverridePolicy::Reject => {
                        return Err(ZoneError::ConflictsWithDevice(name.clone()));
                    }
                    OverridePolicy::CustomWins => {
                        records.clear();
                        overridden.insert(name.clone());
                    }
                }
            }
            records.push(record.clone());
        }

        for (name, records) in &mut nodes {
            normalize(records);
            let has_cname = records.iter().any(|r| r.record_type() == RecordType::Cname);
            if has_cname && records.len() > 1 {
                return Err(ZoneError::CnameNotExclusive(name.clone()));
            }
            if has_cname && self.suffixes.contains(name) {
                return Err(ZoneError::CnameAtApex(name.clone()));
            }
            if records.len() > MAX_RECORDS_PER_NAME {
                return Err(ZoneError::TooManyRecordsAtName(name.clone()));
            }
        }

        // Apexes and empty non-terminals exist (NODATA, not NXDOMAIN) and stop wildcards.
        let mut existing: Vec<DomainName> = Vec::new();
        for name in nodes.keys() {
            let Some(apex) = self.apex_of(name.as_str()) else {
                continue;
            };
            for ancestor in self_and_ancestors(name.as_str()).skip(1) {
                if ancestor.len() < apex.as_str().len() {
                    break;
                }
                if let Ok(ancestor) = DomainName::new(ancestor) {
                    existing.push(ancestor);
                }
            }
        }
        existing.extend(self.suffixes.iter().cloned());
        for name in existing {
            nodes.entry(name).or_default();
        }

        let mut reverse = BTreeMap::new();
        for (label, addresses) in &self.devices {
            let target = primary
                .child(label.as_str())
                .map_err(ZoneError::DeviceNameTooLong)?;
            for address in addresses {
                if reverse
                    .insert(reverse_name(*address), target.clone())
                    .is_some()
                {
                    return Err(ZoneError::DuplicateAddress(*address));
                }
            }
        }

        Ok(Zone {
            inner: Arc::new(ZoneInner {
                apexes: self.suffixes.iter().cloned().collect(),
                nodes,
                reverse,
                device_ttl,
                negative_ttl,
                serial: self.serial,
            }),
        })
    }

    fn is_device_name(&self, name: &DomainName) -> bool {
        name.as_str().split_once('.').is_some_and(|(label, rest)| {
            self.devices.contains_key(label) && self.suffixes.iter().any(|s| s.as_str() == rest)
        })
    }

    fn apex_of(&self, name: &str) -> Option<&DomainName> {
        self.suffixes
            .iter()
            .filter(|suffix| is_within(name, suffix.as_str()))
            .max_by_key(|suffix| suffix.as_str().len())
    }
}

fn checked_ttl(ttl: u32) -> Result<u32, ZoneError> {
    if ttl > MAX_TTL {
        Err(ZoneError::TtlTooLarge(ttl))
    } else {
        Ok(ttl)
    }
}

/// Sorts, merges duplicates and gives each `RRset` its lowest TTL.
fn normalize(records: &mut Vec<Record>) {
    records.sort();
    records.dedup_by(|later, earlier| later.data() == earlier.data());
    let mut index = 0;
    while let Some(first) = records.get(index) {
        let kind = first.record_type();
        let end = records
            .iter()
            .skip(index)
            .position(|r| r.record_type() != kind)
            .map_or(records.len(), |offset| index.saturating_add(offset));
        if let Some(rrset) = records.get_mut(index..end) {
            let lowest = rrset.iter().map(Record::ttl).min().unwrap_or(0);
            for record in rrset.iter_mut() {
                *record = record.with_ttl(lowest);
            }
        }
        index = end;
    }
}

/// `d.c.b.a.in-addr.arpa` or the 32-nibble `ip6.arpa` name for an address.
fn reverse_name(address: IpAddr) -> DomainName {
    let mut text = String::with_capacity(72);
    match address {
        IpAddr::V4(v4) => {
            for octet in v4.octets().iter().rev() {
                let _ = write!(text, "{octet}.");
            }
            text.push_str("in-addr.arpa");
        }
        IpAddr::V6(v6) => {
            for octet in v6.octets().iter().rev() {
                let _ = write!(text, "{:x}.{:x}.", octet % 16, octet / 16);
            }
            text.push_str("ip6.arpa");
        }
    }
    #[expect(
        clippy::expect_used,
        reason = "reverse names are digit/hex labels well under 253 bytes"
    )]
    DomainName::new(&text).expect("reverse names are always valid")
}

/// An immutable, validated zone. Cloning is an `Arc` bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    inner: Arc<ZoneInner>,
}

#[derive(Debug, PartialEq, Eq)]
struct ZoneInner {
    apexes: BTreeSet<DomainName>,
    /// Every existing name: records, devices, apexes and empty non-terminals (empty vec).
    nodes: BTreeMap<DomainName, Vec<Record>>,
    reverse: BTreeMap<DomainName, DomainName>,
    device_ttl: u32,
    negative_ttl: u32,
    serial: u32,
}

impl Zone {
    /// The authoritative suffixes, sorted.
    pub fn suffixes(&self) -> impl Iterator<Item = &DomainName> {
        self.inner.apexes.iter()
    }

    /// The records stored at exactly `name` (no wildcard synthesis), sorted by data.
    /// `Some(&[])` means the name exists with no records (apex or empty non-terminal).
    pub fn records(&self, name: &DomainName) -> Option<&[Record]> {
        self.inner.nodes.get(name).map(Vec::as_slice)
    }

    /// The SOA serial.
    pub fn serial(&self) -> u32 {
        self.inner.serial
    }

    pub(crate) fn negative_ttl(&self) -> u32 {
        self.inner.negative_ttl
    }

    pub(crate) fn device_ttl(&self) -> u32 {
        self.inner.device_ttl
    }

    pub(crate) fn is_apex(&self, name: &str) -> bool {
        self.inner.apexes.contains(name)
    }

    pub(crate) fn reverse_target(&self, name: &str) -> Option<&DomainName> {
        self.inner.reverse.get(name)
    }

    /// The records answering `name` within `apex`, synthesizing from a wildcard per
    /// RFC 4592 when `name` does not exist. `None` means NXDOMAIN.
    pub(crate) fn find(&self, name: &str, apex: &str) -> Option<&[Record]> {
        if let Some(records) = self.inner.nodes.get(name) {
            return Some(records);
        }
        // The closest encloser is the longest existing ancestor; only `*.<encloser>`
        // may synthesize the answer.
        let encloser = self_and_ancestors(name)
            .skip(1)
            .take_while(|ancestor| ancestor.len() >= apex.len())
            .find(|ancestor| self.inner.nodes.contains_key(*ancestor))?;
        let mut buffer = [0_u8; 256];
        let wildcard = wildcard_of(&mut buffer, encloser)?;
        self.inner.nodes.get(wildcard).map(Vec::as_slice)
    }
}

/// Writes `*.<encloser>` into `buffer` without allocating.
fn wildcard_of<'b>(buffer: &'b mut [u8; 256], encloser: &str) -> Option<&'b str> {
    let length = encloser.len().checked_add(2)?;
    let target = buffer.get_mut(..length)?;
    let (prefix, rest) = target.split_at_mut_checked(2)?;
    prefix.copy_from_slice(b"*.");
    rest.copy_from_slice(encloser.as_bytes());
    core::str::from_utf8(target).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::net::Ipv4Addr;

    fn name(text: &str) -> DomainName {
        DomainName::new(text).unwrap()
    }

    fn a(last: u8) -> Record {
        Record::new(60, RecordData::A(Ipv4Addr::new(100, 64, 0, last))).unwrap()
    }

    fn cname(target: &str) -> Record {
        Record::new(60, RecordData::Cname(name(target))).unwrap()
    }

    fn base() -> ZoneBuilder {
        let mut builder = ZoneBuilder::new();
        builder.add_suffix(name("home.example.com")).unwrap();
        builder
            .add_device("laptop", &[IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))])
            .unwrap();
        builder
    }

    #[test]
    fn build_rejects_cname_conflicts_out_of_zone_and_device_hijack() {
        let mut shared = base();
        shared
            .add_record(name("www.home.example.com"), cname("x.com"))
            .unwrap();
        shared
            .add_record(name("www.home.example.com"), a(9))
            .unwrap();
        assert_eq!(
            shared.build(),
            Err(ZoneError::CnameNotExclusive(name("www.home.example.com")))
        );

        let mut apex = base();
        apex.add_record(name("home.example.com"), cname("x.com"))
            .unwrap();
        assert_eq!(
            apex.build(),
            Err(ZoneError::CnameAtApex(name("home.example.com")))
        );

        let mut outside = base();
        outside.add_record(name("example.com"), a(9)).unwrap();
        assert_eq!(
            outside.build(),
            Err(ZoneError::OutsideZone(name("example.com")))
        );

        let mut hijack = base();
        hijack
            .add_record(name("laptop.home.example.com"), cname("evil.com"))
            .unwrap();
        assert_eq!(
            hijack.build(),
            Err(ZoneError::ConflictsWithDevice(name(
                "laptop.home.example.com"
            )))
        );

        // With CustomWins the CNAME replaces the device's A record, so exclusivity holds.
        hijack.override_policy(OverridePolicy::CustomWins);
        let zone = hijack.build().unwrap();
        assert_eq!(
            zone.records(&name("laptop.home.example.com")).unwrap(),
            [cname("evil.com")]
        );

        assert!(matches!(
            base().add_device("a.b", &[]),
            Err(ZoneError::InvalidDeviceName(_))
        ));
        assert!(matches!(
            base().add_device("_x", &[]),
            Err(ZoneError::InvalidDeviceName(_))
        ));
    }

    #[test]
    fn zone_is_identical_regardless_of_insertion_order_and_merges_rrset_ttls() {
        let records = [
            (name("a.home.example.com"), a(1)),
            (
                name("a.home.example.com"),
                Record::new(10, RecordData::A(Ipv4Addr::new(100, 64, 0, 2))).unwrap(),
            ),
            (name("a.home.example.com"), a(1)),
            (name("b.c.home.example.com"), a(3)),
        ];
        let mut forward = base();
        let mut backward = base();
        for (owner, record) in &records {
            forward.add_record(owner.clone(), record.clone()).unwrap();
        }
        for (owner, record) in records.iter().rev() {
            backward.add_record(owner.clone(), record.clone()).unwrap();
        }
        let zone = forward.build().unwrap();
        assert_eq!(zone, backward.build().unwrap());
        let rrset = zone.records(&name("a.home.example.com")).unwrap();
        assert_eq!(rrset.len(), 2);
        assert!(rrset.iter().all(|r| r.ttl() == 10));
        // The empty non-terminal exists, so wildcards cannot cover it.
        assert_eq!(zone.records(&name("c.home.example.com")), Some(&[][..]));
    }
}
