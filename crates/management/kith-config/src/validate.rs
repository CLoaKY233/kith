use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;

use crate::error::{ValidationError, ValidationErrors};
use crate::schema::{
    AmneziaParams, Config, DaemonConfig, DnsConfig, DnsOverride, ExitConfig, NetworkConfig,
    Obfuscation, ServerConfig, StealthConfig,
};

/// AmneziaWG limits: junk and padding must still fit a 1280-byte path.
const MAX_JUNK_COUNT: u8 = 128;
const MAX_JUNK_SIZE: u16 = 1280;
const MAX_INIT_PADDING: u16 = 1280 - 148;
const MAX_RESPONSE_PADDING: u16 = 1280 - 92;
/// Initiation (148) and response (92) differ by 56 bytes; equal padded sizes would make them
/// indistinguishable to the receiver.
const INIT_RESPONSE_SIZE_DELTA: u16 = 56;
const MIN_WS_PATH_SECRET: usize = 8;
const MAX_NAME: usize = 64;

#[derive(Default)]
struct Report(Vec<ValidationError>);

impl Report {
    fn fail(&mut self, field: &str, message: impl Into<String>) {
        self.0.push(ValidationError {
            field: field.to_owned(),
            message: message.into(),
        });
    }

    fn require(&mut self, ok: bool, field: &str, message: &str) {
        if !ok {
            self.fail(field, message);
        }
    }

    fn hostname(&mut self, value: Option<&str>, field: &str) {
        if let Some(value) = value {
            self.require(is_hostname(value), field, "must be a DNS hostname");
        }
    }

    fn absolute(&mut self, value: Option<&Path>, field: &str) {
        if let Some(value) = value {
            self.require(value.is_absolute(), field, "must be an absolute path");
        }
    }

    fn listener(&mut self, value: Option<SocketAddr>, field: &str) {
        if let Some(value) = value {
            self.require(value.port() != 0, field, "port must not be 0");
        }
    }
}

impl Config {
    /// Checks cross-field rules, returning every violation found.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut report = Report::default();
        daemon(&self.daemon, &mut report);
        network(&self.network, &mut report);
        stealth(&self.stealth, &mut report);
        server(&self.server, &mut report);
        dns(&self.dns, &mut report);
        self.exit_and_imports(&mut report);
        if report.0.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(report.0))
        }
    }

    fn exit_and_imports(&self, report: &mut Report) {
        let mut names = BTreeSet::new();
        for import in &self.import_wireguard {
            report.require(
                is_name(&import.name),
                "import_wireguard.name",
                "must be 1-64 of [a-zA-Z0-9_-]",
            );
            if !names.insert(import.name.as_str()) {
                report.fail(
                    "import_wireguard.name",
                    format!("`{}` is used twice", import.name),
                );
            }
            report.absolute(Some(&import.path), "import_wireguard.path");
        }
        let ExitConfig {
            use_exit,
            allow_lan: _,
        } = &self.exit;
        let Some(selector) = use_exit else { return };
        match (&selector.node, &selector.wireguard) {
            (Some(node), None) => {
                report.require(!node.is_empty(), "exit.use_exit.node", "must not be empty");
            }
            (None, Some(name)) => {
                report.require(
                    names.contains(name.as_str()),
                    "exit.use_exit.wireguard",
                    "must name an [[import_wireguard]] entry",
                );
                report.require(
                    selector.network.is_none(),
                    "exit.use_exit.network",
                    "only applies to `node`",
                );
            }
            (Some(_), Some(_)) | (None, None) => {
                report.fail("exit.use_exit", "set exactly one of `node` or `wireguard`");
            }
        }
        if self.server.exit {
            report.fail(
                "exit.use_exit",
                "conflicts with server.exit: an exit cannot use another exit",
            );
        }
    }
}

fn daemon(config: &DaemonConfig, report: &mut Report) {
    report.absolute(Some(&config.state_dir), "daemon.state_dir");
    report.absolute(Some(&config.socket_path), "daemon.socket_path");
    report.require(
        config.socket_path.as_os_str().len() <= MAX_SOCKET_PATH,
        "daemon.socket_path",
        "must be at most 103 bytes, the limit for Unix socket paths on macOS",
    );
}

const MAX_SOCKET_PATH: usize = 103;

fn network(config: &NetworkConfig, report: &mut Report) {
    report.require(
        (NetworkConfig::MIN_MTU..=NetworkConfig::MAX_MTU).contains(&config.mtu),
        "network.mtu",
        "must be in 1280..=9000",
    );
    if let Some(name) = &config.tun_name {
        let portable = !name.is_empty()
            && name.len() <= 15
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        report.require(
            portable,
            "network.tun_name",
            "must be 1-15 of [a-zA-Z0-9_-]",
        );
        if cfg!(target_os = "macos") {
            let utun = name
                .strip_prefix("utun")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            report.require(utun, "network.tun_name", "must be `utunN` on macOS");
        }
    }
    let order = &config.transports.order;
    report.require(
        !order.is_empty(),
        "network.transports.order",
        "must enable at least one transport",
    );
    if order.iter().collect::<BTreeSet<_>>().len() != order.len() {
        report.fail("network.transports.order", "lists a transport twice");
    }
    if config.transports.stealth_only && !order.iter().any(|kind| kind.is_stealth()) {
        report.fail(
            "network.transports.stealth_only",
            "needs `quic` or `websocket` in order",
        );
    }
}

fn stealth(config: &StealthConfig, report: &mut Report) {
    report.hostname(config.quic_sni.as_deref(), "stealth.quic_sni");
    report.hostname(config.cdn_front.as_deref(), "stealth.cdn_front");
    for url in &config.websocket_urls {
        let host = url
            .strip_prefix("wss://")
            .or_else(|| url.strip_prefix("ws://"))
            .map(|rest| rest.split(['/', ':']).next().unwrap_or_default());
        report.require(
            host.is_some_and(|host| !host.is_empty()),
            "stealth.websocket_urls",
            "must be ws:// or wss:// URLs with a host",
        );
    }
    if let Some(params) = &config.amnezia {
        if config.obfuscation == Obfuscation::Off {
            report.fail("stealth.amnezia", "conflicts with obfuscation = \"off\"");
        }
        amnezia(params, report);
    }
}

fn amnezia(params: &AmneziaParams, report: &mut Report) {
    report.require(
        params.junk_packet_count <= MAX_JUNK_COUNT,
        "stealth.amnezia.junk_packet_count",
        "must be at most 128",
    );
    report.require(
        params.junk_packet_min_size <= params.junk_packet_max_size
            && params.junk_packet_max_size <= MAX_JUNK_SIZE,
        "stealth.amnezia.junk_packet_max_size",
        "needs junk_packet_min_size <= junk_packet_max_size <= 1280",
    );
    report.require(
        params.init_padding <= MAX_INIT_PADDING,
        "stealth.amnezia.init_padding",
        "must be at most 1132",
    );
    report.require(
        params.response_padding <= MAX_RESPONSE_PADDING,
        "stealth.amnezia.response_padding",
        "must be at most 1188",
    );
    report.require(
        params.init_padding.checked_add(INIT_RESPONSE_SIZE_DELTA) != Some(params.response_padding),
        "stealth.amnezia.response_padding",
        "must not equal init_padding + 56 (initiation and response would look alike)",
    );
    let headers = [
        params.init_header,
        params.response_header,
        params.cookie_header,
        params.transport_header,
    ];
    report.require(
        headers.iter().collect::<BTreeSet<_>>().len() == headers.len(),
        "stealth.amnezia",
        "the four headers must be distinct",
    );
}

fn server(config: &ServerConfig, report: &mut Report) {
    report.hostname(config.public_hostname.as_deref(), "server.public_hostname");
    report.listener(config.ws_listen, "server.ws_listen");
    report.listener(config.quic_listen, "server.quic_listen");
    report.absolute(config.tls_cert.as_deref(), "server.tls_cert");
    report.absolute(config.tls_key.as_deref(), "server.tls_key");
    if config.tls_cert.is_some() != config.tls_key.is_some() {
        report.fail(
            "server.tls_cert",
            "tls_cert and tls_key must be set together",
        );
    }
    if let Some(path) = &config.ws_path {
        let secret = path.strip_prefix('/').unwrap_or_default();
        report.require(
            path.starts_with('/')
                && secret.len() >= MIN_WS_PATH_SECRET
                && path.bytes().all(|b| b.is_ascii_graphic()),
            "server.ws_path",
            "must be `/` followed by at least 8 printable characters",
        );
    }
    let configured =
        config.exit || config.relay || config.ws_listen.is_some() || config.quic_listen.is_some();
    if config.enabled {
        report.require(
            configured,
            "server",
            "server mode needs exit, relay, ws_listen or quic_listen",
        );
    } else {
        report.require(
            !configured,
            "server.enabled",
            "server options are set but server mode is off",
        );
    }
}

fn dns(config: &DnsConfig, report: &mut Report) {
    if !config.enabled && config.override_system != DnsOverride::Off {
        report.fail(
            "dns.override_system",
            "must be \"off\" when dns.enabled = false",
        );
    }
}

pub(crate) fn is_hostname(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && text.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
