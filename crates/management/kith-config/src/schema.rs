use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Config files larger than this are rejected before parsing.
pub(crate) const MAX_FILE_BYTES: u64 = 1 << 20;

/// The daemon configuration. `Default` is the zero-config behaviour.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Process-level settings.
    pub daemon: DaemonConfig,
    /// Local socket and interface settings.
    pub network: NetworkConfig,
    /// Censorship-resistance settings.
    pub stealth: StealthConfig,
    /// Server mode (`kith server`).
    pub server: ServerConfig,
    /// Mesh DNS and OS resolver integration.
    pub dns: DnsConfig,
    /// Exit-node selection.
    pub exit: ExitConfig,
    /// Standard WireGuard configs to use as exits.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub import_wireguard: Vec<ImportWireguard>,
}

impl Config {
    /// Parses and validates a TOML document.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text).map_err(|e| ConfigError::Parse(Box::new(e)))?;
        config.validate().map_err(ConfigError::Invalid)?;
        Ok(config)
    }

    /// Reads, parses and validates the file at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::from_toml_str(&read_bounded(path, MAX_FILE_BYTES)?)
    }

    /// Like [`Config::load`], but a missing file means zero config.
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        match Self::load(path) {
            Err(ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Self::default())
            }
            other => other,
        }
    }

    /// Renders the configuration as TOML, for `kith config show`.
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string(self)?)
    }

    /// Where this OS keeps the system-wide config file.
    pub fn default_path() -> PathBuf {
        if cfg!(target_os = "macos") {
            PathBuf::from("/Library/Application Support/kith/kith.toml")
        } else if cfg!(windows) {
            program_data().join("kith").join("kith.toml")
        } else {
            PathBuf::from("/etc/kith/kith.toml")
        }
    }
}

/// `%ProgramData%`, falling back to its usual location when unset or not absolute.
fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<String, ConfigError> {
    let read_error = |source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    };
    let length = std::fs::metadata(path).map_err(read_error)?.len();
    if length > limit {
        return Err(ConfigError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    std::fs::read_to_string(path).map_err(read_error)
}

/// Process-level settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    /// Where keys, the ledger and caches live.
    pub state_dir: PathBuf,
    /// The IPC socket the CLI talks to.
    pub socket_path: PathBuf,
    /// Log verbosity.
    pub log_level: LogLevel,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let (state_dir, socket_path) = if cfg!(target_os = "linux") {
            ("/var/lib/kith".into(), "/run/kith/kith.sock")
        } else if cfg!(target_os = "macos") {
            (
                "/Library/Application Support/kith".into(),
                "/var/run/kith/kith.sock",
            )
        } else if cfg!(windows) {
            (program_data().join("kith"), r"\\.\pipe\kith")
        } else {
            ("/var/db/kith".into(), "/var/run/kith/kith.sock")
        };
        Self {
            state_dir,
            socket_path: PathBuf::from(socket_path),
            log_level: LogLevel::default(),
        }
    }
}

/// Log verbosity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Errors and warnings.
    Warn,
    /// Normal operation.
    #[default]
    Info,
    /// Diagnostics.
    Debug,
    /// Everything.
    Trace,
}

/// Local socket and interface settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// UDP port for WireGuard; 0 picks a random one.
    ///
    /// Fixed by default so NAT mappings and cached peer endpoints survive restarts and users can
    /// port-forward or allow-list it.
    pub listen_port: u16,
    /// TUN interface name; `None` uses the OS default (`kith0`, or the next free `utunN`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tun_name: Option<String>,
    /// Tunnel MTU. 1280 is the IPv6 minimum and survives `PPPoE`, mobile and nested tunnels.
    pub mtu: u16,
    /// Which transports to try, and in what order.
    pub transports: TransportsConfig,
}

impl NetworkConfig {
    /// The default WireGuard port.
    pub const DEFAULT_LISTEN_PORT: u16 = 41641;
    /// The default and minimum MTU.
    pub const MIN_MTU: u16 = 1280;
    /// The largest MTU accepted (jumbo frames).
    pub const MAX_MTU: u16 = 9000;
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_port: Self::DEFAULT_LISTEN_PORT,
            tun_name: None,
            mtu: Self::MIN_MTU,
            transports: TransportsConfig::default(),
        }
    }
}

/// Transport selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransportsConfig {
    /// Enabled transports in preference order; a transport not listed is disabled.
    pub order: Vec<TransportKind>,
    /// Never send plain WireGuard UDP, even when it would work.
    pub stealth_only: bool,
}

impl Default for TransportsConfig {
    fn default() -> Self {
        Self {
            order: vec![
                TransportKind::Udp,
                TransportKind::Quic,
                TransportKind::Websocket,
            ],
            stealth_only: false,
        }
    }
}

/// A way to carry WireGuard packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// Plain UDP (optionally obfuscated).
    Udp,
    /// QUIC on 443, looking like HTTP/3.
    Quic,
    /// WebSocket over TLS, CDN-frontable.
    Websocket,
}

impl TransportKind {
    /// Whether the transport hides WireGuard from DPI by itself.
    pub const fn is_stealth(self) -> bool {
        matches!(self, Self::Quic | Self::Websocket)
    }
}

/// Censorship-resistance settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StealthConfig {
    /// Whether UDP packets are obfuscated.
    pub obfuscation: Obfuscation,
    /// TLS SNI presented by the QUIC transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quic_sni: Option<String>,
    /// WebSocket endpoints (`wss://host/path`), tried in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub websocket_urls: Vec<String>,
    /// Domain fronted through a CDN for the WebSocket transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdn_front: Option<String>,
    /// Explicit AmneziaWG parameters; `None` derives them from the network ledger.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amnezia: Option<AmneziaParams>,
}

/// Whether UDP packets are obfuscated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Obfuscation {
    /// Turned on when plain WireGuard appears blocked.
    #[default]
    Auto,
    /// Never.
    Off,
    /// Always.
    On,
}

/// AmneziaWG-style obfuscation parameters, as plain numbers for `kith-obfs` to map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmneziaParams {
    /// Junk packets sent before a handshake (`Jc`).
    pub junk_packet_count: u8,
    /// Smallest junk packet in bytes (`Jmin`).
    pub junk_packet_min_size: u16,
    /// Largest junk packet in bytes (`Jmax`).
    pub junk_packet_max_size: u16,
    /// Padding prepended to handshake initiations (`S1`).
    pub init_padding: u16,
    /// Padding prepended to handshake responses (`S2`).
    pub response_padding: u16,
    /// Message type replacing handshake initiation (`H1`).
    pub init_header: u32,
    /// Message type replacing handshake response (`H2`).
    pub response_header: u32,
    /// Message type replacing cookie reply (`H3`).
    pub cookie_header: u32,
    /// Message type replacing transport data (`H4`).
    pub transport_header: u32,
}

/// Server mode (`kith server --exit --stealth`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Run as an always-on server.
    pub enabled: bool,
    /// Offer this node as an exit.
    pub exit: bool,
    /// Relay traffic for peers that cannot connect directly.
    pub relay: bool,
    /// Name clients use to reach this server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_hostname: Option<String>,
    /// WebSocket listener address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_listen: Option<SocketAddr>,
    /// QUIC listener address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quic_listen: Option<SocketAddr>,
    /// TLS certificate chain (PEM); omit behind a TLS-terminating CDN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_cert: Option<PathBuf>,
    /// TLS private key (PEM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_key: Option<PathBuf>,
    /// Secret WebSocket path; other paths get an ordinary web response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_path: Option<String>,
}

/// Mesh DNS and OS resolver integration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DnsConfig {
    /// Serve names for mesh nodes.
    pub enabled: bool,
    /// How the OS resolver is changed.
    pub override_system: DnsOverride,
    /// Upstreams for queries kith does not answer itself.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallback_upstreams: Vec<Upstream>,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            override_system: DnsOverride::Split,
            fallback_upstreams: Vec::new(),
        }
    }
}

/// How the OS resolver is changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsOverride {
    /// Only mesh domains go to kith.
    #[default]
    Split,
    /// All queries go to kith.
    Full,
    /// The OS resolver is untouched.
    Off,
}

/// A DNS upstream; the port defaults to 53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Upstream(pub SocketAddr);

impl FromStr for Upstream {
    type Err = std::net::AddrParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse::<SocketAddr>()
            .or_else(|_| text.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, 53)))
            .map(Self)
    }
}

impl TryFrom<String> for Upstream {
    type Error = std::net::AddrParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Upstream> for String {
    fn from(upstream: Upstream) -> Self {
        upstream.to_string()
    }
}

impl fmt::Display for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Exit-node selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExitConfig {
    /// The exit to route internet traffic through; `None` routes it locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_exit: Option<ExitSelector>,
    /// Keep the local LAN reachable while using an exit.
    pub allow_lan: bool,
}

/// Names an exit: a mesh node (`node`, optionally scoped by `network`) or an imported config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitSelector {
    /// Network name; needed only when several networks have a node of that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// Node name or id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Name of an `[[import_wireguard]]` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wireguard: Option<String>,
}

/// A standard wg-quick `.conf` (Mullvad, Proton, ...) usable as an exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportWireguard {
    /// Name used by `exit.use_exit.wireguard`.
    pub name: String,
    /// Absolute path to the `.conf`.
    pub path: PathBuf,
}

impl ImportWireguard {
    /// Reads and parses the referenced wg-quick file.
    pub fn load(&self) -> Result<crate::WgQuickConfig, ConfigError> {
        crate::WgQuickConfig::load(&self.path)
    }
}
