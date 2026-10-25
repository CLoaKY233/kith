//! A strict parser for wg-quick `.conf` files.
//!
//! Unknown keys are errors. Hook keys (`PostUp` and friends) are rejected rather than ignored,
//! because a provider's kill switch living in a hook would otherwise silently vanish.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use data_encoding::BASE64;
use kith_types::{IpPrefix, PrefixError};
use zeroize::Zeroize;

use crate::error::ConfigError;
use crate::schema::read_bounded;
use crate::validate::is_hostname;

const MAX_FILE_BYTES: usize = 64 * 1024;
const KEY_LEN: usize = 32;
const HOOK_KEYS: [&str; 5] = ["PreUp", "PostUp", "PreDown", "PostDown", "SaveConfig"];

/// A parsed wg-quick file.
#[derive(Debug, PartialEq, Eq)]
pub struct WgQuickConfig {
    /// The `[Interface]` section.
    pub interface: WgInterface,
    /// The `[Peer]` sections, in file order; at least one.
    pub peers: Vec<WgPeer>,
}

/// The `[Interface]` section.
#[derive(Debug, PartialEq, Eq)]
pub struct WgInterface {
    /// `PrivateKey`.
    pub private_key: WgSecret,
    /// `Address`, possibly repeated.
    pub addresses: Vec<InterfaceAddress>,
    /// IP entries of `DNS`.
    pub dns: Vec<IpAddr>,
    /// Non-IP entries of `DNS`, which wg-quick treats as search domains.
    pub search_domains: Vec<String>,
    /// `MTU`.
    pub mtu: Option<u16>,
    /// `ListenPort`.
    pub listen_port: Option<u16>,
}

/// A `[Peer]` section.
#[derive(Debug, PartialEq, Eq)]
pub struct WgPeer {
    /// `PublicKey`.
    pub public_key: WgPublicKey,
    /// `PresharedKey`.
    pub preshared_key: Option<WgSecret>,
    /// `Endpoint`.
    pub endpoint: Option<Endpoint>,
    /// `AllowedIPs`, possibly repeated.
    pub allowed_ips: Vec<IpPrefix>,
    /// `PersistentKeepalive` in seconds; `None` when off.
    pub persistent_keepalive: Option<u16>,
}

/// An interface address; unlike a route prefix, host bits are expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceAddress {
    /// The address.
    pub addr: IpAddr,
    /// The on-link prefix length.
    pub prefix_len: u8,
}

/// A peer endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A literal socket address.
    Addr(SocketAddr),
    /// A hostname to resolve.
    Host {
        /// The hostname.
        host: String,
        /// The UDP port.
        port: u16,
    },
}

/// A WireGuard public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WgPublicKey(pub [u8; KEY_LEN]);

impl fmt::Display for WgPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&BASE64.encode(&self.0))
    }
}

impl fmt::Debug for WgPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A private or preshared key: not `Clone`, redacted in `Debug`, zeroized on drop.
#[derive(PartialEq, Eq)]
pub struct WgSecret([u8; KEY_LEN]);

impl WgSecret {
    /// The raw key bytes.
    pub const fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for WgSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Drop for WgSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A wg-quick parse failure at a 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {kind}")]
pub struct WgQuickError {
    /// 1-based line number (0 when the problem is the file as a whole).
    pub line: usize,
    /// What went wrong.
    pub kind: WgQuickErrorKind,
}

/// What went wrong in a wg-quick file. Messages never contain key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WgQuickErrorKind {
    /// The file exceeds 64 KiB.
    #[error("file is larger than 64 KiB")]
    TooLarge,
    /// A line is neither a section header nor `Key = Value`.
    #[error("expected `[Section]` or `Key = Value`")]
    Syntax,
    /// A section other than `[Interface]` or `[Peer]`.
    #[error("unknown section `[{0}]`")]
    UnknownSection(String),
    /// A key before any section.
    #[error("key outside of a section")]
    KeyOutsideSection,
    /// A key this parser does not know.
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    /// A hook or state key kith deliberately does not honour.
    #[error("`{0}` is not supported: kith does not run wg-quick hooks")]
    UnsupportedKey(String),
    /// A single-valued key given twice.
    #[error("duplicate key `{0}`")]
    DuplicateKey(String),
    /// A required key is absent from the section starting at this line.
    #[error("missing `{0}`")]
    MissingKey(&'static str),
    /// More than one `[Interface]`.
    #[error("second [Interface] section")]
    DuplicateInterface,
    /// No `[Interface]`.
    #[error("no [Interface] section")]
    MissingInterface,
    /// No `[Peer]`.
    #[error("no [Peer] section")]
    MissingPeer,
    /// A key value is not 32 bytes of base64.
    #[error("`{0}` is not a base64 32-byte key")]
    InvalidKey(&'static str),
    /// A bad `AllowedIPs` entry.
    #[error("invalid AllowedIPs entry `{value}`: {reason}")]
    InvalidPrefix {
        /// The entry.
        value: String,
        /// Why.
        reason: PrefixError,
    },
    /// A bad `Address` entry.
    #[error("invalid Address entry `{0}`")]
    InvalidAddress(String),
    /// A bad `DNS` entry.
    #[error("invalid DNS entry `{0}`")]
    InvalidDns(String),
    /// A bad `Endpoint`.
    #[error("invalid Endpoint `{0}`")]
    InvalidEndpoint(String),
    /// A bad number, or one out of range.
    #[error("invalid value for `{0}`")]
    InvalidNumber(&'static str),
}

#[derive(Default)]
struct InterfaceDraft {
    line: usize,
    private_key: Option<WgSecret>,
    addresses: Vec<InterfaceAddress>,
    dns: Vec<IpAddr>,
    search_domains: Vec<String>,
    mtu: Option<u16>,
    listen_port: Option<u16>,
}

#[derive(Default)]
struct PeerDraft {
    line: usize,
    public_key: Option<WgPublicKey>,
    preshared_key: Option<WgSecret>,
    endpoint: Option<Endpoint>,
    allowed_ips: Vec<IpPrefix>,
    persistent_keepalive: Option<u16>,
    keepalive_seen: bool,
}

enum Section {
    None,
    Interface,
    Peer,
}

impl WgQuickConfig {
    /// Reads and parses a wg-quick file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let limit = u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX);
        let text = read_bounded(path, limit)?;
        Self::parse(&text).map_err(|source| ConfigError::WgQuick {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Parses wg-quick text.
    pub fn parse(text: &str) -> Result<Self, WgQuickError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(WgQuickError {
                line: 0,
                kind: WgQuickErrorKind::TooLarge,
            });
        }
        let mut interface: Option<InterfaceDraft> = None;
        let mut peers: Vec<PeerDraft> = Vec::new();
        let mut section = Section::None;
        for (index, raw) in text.lines().enumerate() {
            let line = index.saturating_add(1);
            let at = |kind| WgQuickError { line, kind };
            let content = raw.split('#').next().unwrap_or_default().trim();
            if content.is_empty() {
                continue;
            }
            if let Some(name) = content
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                let name = name.trim();
                if name.eq_ignore_ascii_case("Interface") {
                    if interface.is_some() {
                        return Err(at(WgQuickErrorKind::DuplicateInterface));
                    }
                    interface = Some(InterfaceDraft {
                        line,
                        ..InterfaceDraft::default()
                    });
                    section = Section::Interface;
                } else if name.eq_ignore_ascii_case("Peer") {
                    peers.push(PeerDraft {
                        line,
                        ..PeerDraft::default()
                    });
                    section = Section::Peer;
                } else {
                    return Err(at(WgQuickErrorKind::UnknownSection(name.to_owned())));
                }
                continue;
            }
            let (key, value) = content
                .split_once('=')
                .ok_or_else(|| at(WgQuickErrorKind::Syntax))?;
            let (key, value) = (key.trim(), value.trim());
            if HOOK_KEYS.iter().any(|hook| hook.eq_ignore_ascii_case(key)) {
                return Err(at(WgQuickErrorKind::UnsupportedKey(key.to_owned())));
            }
            let result = match section {
                Section::None => Err(WgQuickErrorKind::KeyOutsideSection),
                Section::Interface => match interface.as_mut() {
                    Some(draft) => interface_key(draft, key, value),
                    None => Err(WgQuickErrorKind::KeyOutsideSection),
                },
                Section::Peer => match peers.last_mut() {
                    Some(draft) => peer_key(draft, key, value),
                    None => Err(WgQuickErrorKind::KeyOutsideSection),
                },
            };
            result.map_err(at)?;
        }
        finish(interface, peers)
    }
}

fn finish(
    interface: Option<InterfaceDraft>,
    peers: Vec<PeerDraft>,
) -> Result<WgQuickConfig, WgQuickError> {
    let interface = interface.ok_or(WgQuickError {
        line: 0,
        kind: WgQuickErrorKind::MissingInterface,
    })?;
    if peers.is_empty() {
        return Err(WgQuickError {
            line: 0,
            kind: WgQuickErrorKind::MissingPeer,
        });
    }
    let missing = |line, key| WgQuickError {
        line,
        kind: WgQuickErrorKind::MissingKey(key),
    };
    let interface_line = interface.line;
    let interface = WgInterface {
        private_key: interface
            .private_key
            .ok_or_else(|| missing(interface_line, "PrivateKey"))?,
        addresses: interface.addresses,
        dns: interface.dns,
        search_domains: interface.search_domains,
        mtu: interface.mtu,
        listen_port: interface.listen_port,
    };
    let peers = peers
        .into_iter()
        .map(|peer| {
            Ok(WgPeer {
                public_key: peer
                    .public_key
                    .ok_or_else(|| missing(peer.line, "PublicKey"))?,
                preshared_key: peer.preshared_key,
                endpoint: peer.endpoint,
                allowed_ips: peer.allowed_ips,
                persistent_keepalive: peer.persistent_keepalive,
            })
        })
        .collect::<Result<_, WgQuickError>>()?;
    Ok(WgQuickConfig { interface, peers })
}

fn interface_key(
    draft: &mut InterfaceDraft,
    key: &str,
    value: &str,
) -> Result<(), WgQuickErrorKind> {
    if key.eq_ignore_ascii_case("PrivateKey") {
        set_once(&mut draft.private_key, key, || {
            decode_secret(value, "PrivateKey")
        })
    } else if key.eq_ignore_ascii_case("Address") {
        for item in list(value) {
            draft.addresses.push(parse_interface_address(item)?);
        }
        Ok(())
    } else if key.eq_ignore_ascii_case("DNS") {
        for item in list(value) {
            if let Ok(ip) = item.parse() {
                draft.dns.push(ip);
            } else if is_hostname(item) {
                draft.search_domains.push(item.to_owned());
            } else {
                return Err(WgQuickErrorKind::InvalidDns(item.to_owned()));
            }
        }
        Ok(())
    } else if key.eq_ignore_ascii_case("MTU") {
        set_once(&mut draft.mtu, key, || {
            value
                .parse::<u16>()
                .ok()
                .filter(|mtu| (1280..=9000).contains(mtu))
                .ok_or(WgQuickErrorKind::InvalidNumber("MTU"))
        })
    } else if key.eq_ignore_ascii_case("ListenPort") {
        set_once(&mut draft.listen_port, key, || {
            value
                .parse()
                .map_err(|_| WgQuickErrorKind::InvalidNumber("ListenPort"))
        })
    } else {
        Err(WgQuickErrorKind::UnknownKey(key.to_owned()))
    }
}

fn peer_key(draft: &mut PeerDraft, key: &str, value: &str) -> Result<(), WgQuickErrorKind> {
    if key.eq_ignore_ascii_case("PublicKey") {
        set_once(&mut draft.public_key, key, || {
            decode_key(value, "PublicKey").map(WgPublicKey)
        })
    } else if key.eq_ignore_ascii_case("PresharedKey") {
        set_once(&mut draft.preshared_key, key, || {
            decode_secret(value, "PresharedKey")
        })
    } else if key.eq_ignore_ascii_case("Endpoint") {
        set_once(&mut draft.endpoint, key, || parse_endpoint(value))
    } else if key.eq_ignore_ascii_case("AllowedIPs") {
        for item in list(value) {
            let prefix = item
                .parse()
                .map_err(|reason| WgQuickErrorKind::InvalidPrefix {
                    value: item.to_owned(),
                    reason,
                })?;
            draft.allowed_ips.push(prefix);
        }
        Ok(())
    } else if key.eq_ignore_ascii_case("PersistentKeepalive") {
        if core::mem::replace(&mut draft.keepalive_seen, true) {
            return Err(WgQuickErrorKind::DuplicateKey(key.to_owned()));
        }
        draft.persistent_keepalive = if value.eq_ignore_ascii_case("off") {
            None
        } else {
            let seconds = value
                .parse::<u16>()
                .map_err(|_| WgQuickErrorKind::InvalidNumber("PersistentKeepalive"))?;
            (seconds != 0).then_some(seconds)
        };
        Ok(())
    } else {
        Err(WgQuickErrorKind::UnknownKey(key.to_owned()))
    }
}

fn set_once<T>(
    slot: &mut Option<T>,
    key: &str,
    parse: impl FnOnce() -> Result<T, WgQuickErrorKind>,
) -> Result<(), WgQuickErrorKind> {
    if slot.is_some() {
        return Err(WgQuickErrorKind::DuplicateKey(key.to_owned()));
    }
    *slot = Some(parse()?);
    Ok(())
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn decode_key(value: &str, key: &'static str) -> Result<[u8; KEY_LEN], WgQuickErrorKind> {
    let invalid = WgQuickErrorKind::InvalidKey(key);
    let mut buffer = [0_u8; KEY_LEN + 1];
    let expected = BASE64
        .decode_len(value.len())
        .map_err(|_| invalid.clone())?;
    let result = buffer
        .get_mut(..expected)
        .and_then(|output| BASE64.decode_mut(value.as_bytes(), output).ok())
        .filter(|&written| written == KEY_LEN)
        .and_then(|_| buffer.get(..KEY_LEN))
        .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes).ok());
    buffer.zeroize();
    result.ok_or(invalid)
}

fn decode_secret(value: &str, key: &'static str) -> Result<WgSecret, WgQuickErrorKind> {
    decode_key(value, key).map(WgSecret)
}

fn parse_interface_address(item: &str) -> Result<InterfaceAddress, WgQuickErrorKind> {
    let invalid = || WgQuickErrorKind::InvalidAddress(item.to_owned());
    let (addr, len) = item
        .split_once('/')
        .map_or((item, None), |(addr, len)| (addr, Some(len)));
    let addr: IpAddr = addr.parse().map_err(|_| invalid())?;
    let width = if addr.is_ipv4() { 32 } else { 128 };
    let prefix_len = match len {
        None => width,
        Some(len) => len
            .parse::<u8>()
            .ok()
            .filter(|&len| len <= width)
            .ok_or_else(invalid)?,
    };
    Ok(InterfaceAddress { addr, prefix_len })
}

fn parse_endpoint(value: &str) -> Result<Endpoint, WgQuickErrorKind> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(Endpoint::Addr(addr));
    }
    value
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
        .filter(|&(host, port)| port != 0 && is_hostname(host))
        .map(|(host, port)| Endpoint::Host {
            host: host.to_owned(),
            port,
        })
        .ok_or_else(|| WgQuickErrorKind::InvalidEndpoint(value.to_owned()))
}
