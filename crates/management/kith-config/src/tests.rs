use std::collections::BTreeSet;
use std::path::PathBuf;

use proptest::prelude::*;

use crate::*;

const EXAMPLE: &str = include_str!("../kith.example.toml");

fn key_paths(table: &toml::Table, prefix: &str, out: &mut BTreeSet<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => key_paths(inner, &path, out),
            toml::Value::Array(items) if matches!(items.first(), Some(toml::Value::Table(_))) => {
                for item in items.iter().filter_map(toml::Value::as_table) {
                    key_paths(item, &path, out);
                }
            }
            toml::Value::String(_)
            | toml::Value::Integer(_)
            | toml::Value::Float(_)
            | toml::Value::Boolean(_)
            | toml::Value::Datetime(_)
            | toml::Value::Array(_) => {
                out.insert(path);
            }
        }
    }
}

/// Uncomments every `#key` / `#[section]` line, keeping one `use_exit` alternative.
fn uncomment_all(text: &str) -> String {
    let mut seen_use_exit = false;
    text.lines()
        .map(|line| match line.strip_prefix('#') {
            Some(rest) if rest.starts_with("use_exit") && seen_use_exit => "",
            Some(rest) if rest.starts_with(|c: char| c.is_ascii_lowercase() || c == '[') => {
                seen_use_exit |= rest.starts_with("use_exit");
                rest
            }
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn example_validates_and_documents_every_setting() {
    let example = Config::from_toml_str(EXAMPLE).unwrap();
    assert_eq!(example.network, NetworkConfig::default());
    assert_eq!(Config::from_toml_str("").unwrap(), Config::default());
    Config::default().validate().unwrap();

    let full_text = uncomment_all(EXAMPLE);
    let full: Config = toml::from_str(&full_text).unwrap();
    let Config {
        daemon:
            DaemonConfig {
                state_dir: _,
                socket_path: _,
                log_level: _,
                operator_group: _,
            },
        network:
            NetworkConfig {
                listen_port: _,
                tun_name,
                mtu: _,
                transports: _,
            },
        stealth:
            StealthConfig {
                obfuscation: _,
                quic_sni,
                websocket_urls,
                cdn_front,
                amnezia,
            },
        server:
            ServerConfig {
                enabled: _,
                exit: _,
                relay: _,
                public_hostname,
                ws_listen,
                quic_listen,
                tls_cert,
                tls_key,
                ws_path,
            },
        dns:
            DnsConfig {
                enabled: _,
                override_system: _,
                fallback_upstreams,
            },
        exit: ExitConfig {
            use_exit,
            allow_lan: _,
        },
        import_wireguard,
    } = &full;
    assert!(tun_name.is_some() && quic_sni.is_some() && cdn_front.is_some() && amnezia.is_some());
    assert!(public_hostname.is_some() && ws_listen.is_some() && quic_listen.is_some());
    assert!(tls_cert.is_some() && tls_key.is_some() && ws_path.is_some() && use_exit.is_some());
    assert!(
        !websocket_urls.is_empty()
            && !fallback_upstreams.is_empty()
            && !import_wireguard.is_empty()
    );

    let mut documented = BTreeSet::new();
    key_paths(&toml::from_str(&full_text).unwrap(), "", &mut documented);
    let mut schema = BTreeSet::new();
    key_paths(
        &toml::from_str(&full.to_toml_string().unwrap()).unwrap(),
        "",
        &mut schema,
    );
    assert_eq!(documented, schema);
}

#[test]
fn typo_reports_key_and_line() {
    let message = Config::from_toml_str("[network]\nlisten_port = 1\nmtuu = 1400\n")
        .unwrap_err()
        .to_string();
    assert!(message.contains("mtuu"), "{message}");
    assert!(message.contains("line 3"), "{message}");
}

#[test]
fn validation_reports_every_error() {
    let long_socket = format!("/run/{}/kith.sock", "d".repeat(100));
    let text = format!(
        "[daemon]\nsocket_path = \"{long_socket}\"\n[network]\nmtu = 100\n[server]\nenabled = true\n\
         tls_cert = \"/c.pem\"\n[dns]\nenabled = false\n[exit]\nuse_exit = {{ wireguard = \"missing\" }}\n"
    );
    let Err(ConfigError::Invalid(errors)) = Config::from_toml_str(&text) else {
        panic!()
    };
    let fields: BTreeSet<_> = errors.0.iter().map(|e| e.field.as_str()).collect();
    let expected = [
        "daemon.socket_path",
        "network.mtu",
        "server",
        "server.tls_cert",
        "dns.override_system",
        "exit.use_exit.wireguard",
    ];
    assert_eq!(fields, expected.into_iter().collect(), "{errors}");

    let relay_only = "[server]\nenabled = true\nrelay = true\n";
    assert!(
        Config::from_toml_str(relay_only).is_ok(),
        "a relay needs no stealth listener"
    );
}

fn hostname() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,8}(\\.[a-z]{2,5}){0,2}"
}

fn abs_path() -> impl Strategy<Value = PathBuf> {
    "(/[a-z]{1,8}){1,3}".prop_map(PathBuf::from)
}

fn listener() -> impl Strategy<Value = std::net::SocketAddr> {
    (any::<[u8; 4]>(), 1_u16..).prop_map(|(ip, port)| (ip, port).into())
}

fn transports() -> impl Strategy<Value = TransportsConfig> {
    let all = [
        TransportKind::Udp,
        TransportKind::Quic,
        TransportKind::Websocket,
    ];
    (
        Just(all.to_vec()).prop_shuffle(),
        1_usize..=3,
        any::<bool>(),
    )
        .prop_map(|(order, n, stealth)| {
            let order: Vec<_> = order.into_iter().take(n).collect();
            let stealth_only = stealth && order.iter().any(|kind| kind.is_stealth());
            TransportsConfig {
                order,
                stealth_only,
            }
        })
}

fn amnezia() -> impl Strategy<Value = AmneziaParams> {
    (
        0_u8..=128,
        0_u16..=1280,
        0_u16..=1280,
        0_u16..=1132,
        0_u16..=1188,
        any::<[u32; 4]>(),
    )
        .prop_filter("distinct headers, sizes differ", |(_, _, _, s1, s2, h)| {
            h.iter().collect::<BTreeSet<_>>().len() == 4 && s1 + 56 != *s2
        })
        .prop_map(|(count, a, b, s1, s2, h)| AmneziaParams {
            junk_packet_count: count,
            junk_packet_min_size: a.min(b),
            junk_packet_max_size: a.max(b),
            init_padding: s1,
            response_padding: s2,
            init_header: h[0],
            response_header: h[1],
            cookie_header: h[2],
            transport_header: h[3],
        })
}

prop_compose! {
    fn config()(
        state_dir in abs_path(), socket_path in abs_path(),
        log_level in prop::sample::select(vec![LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace]),
        listen_port in any::<u16>(), tun in prop::option::of("utun[0-9]{1,2}"), mtu in 1280_u16..=9000,
        transports in transports(),
        obfuscation in prop::sample::select(vec![Obfuscation::Auto, Obfuscation::On]),
        quic_sni in prop::option::of(hostname()), cdn_front in prop::option::of(hostname()),
        websocket_urls in prop::collection::vec(hostname().prop_map(|h| format!("wss://{h}/p")), 0..3),
        amnezia in prop::option::of(amnezia()),
        server_on in any::<bool>(), flags in any::<(bool, bool)>(), hostname in prop::option::of(hostname()),
        ws_listen in prop::option::of(listener()), quic_listen in listener(),
        tls in prop::option::of((abs_path(), abs_path())), ws_path in prop::option::of("/[a-zA-Z0-9]{8,20}"),
        dns_enabled in any::<bool>(), override_system in prop::sample::select(vec![DnsOverride::Split, DnsOverride::Full]),
        upstreams in prop::collection::vec((any::<[u8; 4]>(), any::<u16>()).prop_map(|a| Upstream(a.into())), 0..3),
        imports in prop::collection::btree_map("[a-z]{1,8}", abs_path(), 0..3),
        exit_choice in 0_u8..3, network in prop::option::of("[a-z]{1,8}"), allow_lan in any::<bool>(),
    ) -> Config {
        let import_wireguard: Vec<_> = imports.into_iter().map(|(name, path)| ImportWireguard { name, path }).collect();
        let use_exit = match (exit_choice, import_wireguard.first()) {
            (1, _) => Some(ExitSelector { network, node: Some("vps1".to_owned()), wireguard: None }),
            (2, Some(first)) => Some(ExitSelector { network: None, node: None, wireguard: Some(first.name.clone()) }),
            _ => None,
        };
        let server = if server_on {
            ServerConfig {
                enabled: true, exit: flags.0 && use_exit.is_none(), relay: flags.1, public_hostname: hostname,
                ws_listen, quic_listen: Some(quic_listen),
                tls_cert: tls.as_ref().map(|t| t.0.clone()), tls_key: tls.map(|t| t.1), ws_path,
            }
        } else {
            ServerConfig { public_hostname: hostname, ws_path, ..ServerConfig::default() }
        };
        Config {
            daemon: DaemonConfig { state_dir, socket_path, log_level, operator_group: "wheel".to_owned() },
            network: NetworkConfig { listen_port, tun_name: tun, mtu, transports },
            stealth: StealthConfig { obfuscation, quic_sni, websocket_urls, cdn_front, amnezia },
            server,
            dns: DnsConfig {
                enabled: dns_enabled,
                override_system: if dns_enabled { override_system } else { DnsOverride::Off },
                fallback_upstreams: upstreams,
            },
            exit: ExitConfig { use_exit, allow_lan },
            import_wireguard,
        }
    }
}

proptest! {
    #[test]
    fn rendered_config_parses_back_identically(config in config()) {
        config.validate().unwrap();
        let text = config.to_toml_string().unwrap();
        prop_assert_eq!(Config::from_toml_str(&text).unwrap(), config);
    }

    #[test]
    fn wgquick_parse_never_panics(text in "(\\[(Interface|Peer)\\]|[A-Za-z]{1,12} ?= ?[ -~]{0,40}|[ -~]{0,20})(\n(\\[(Interface|Peer)\\]|[A-Za-z]{1,12} ?= ?[ -~]{0,40}|[ -~]{0,20})){0,12}") {
        drop(WgQuickConfig::parse(&text));
    }
}

const PRIVATE: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
const PUBLIC: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

#[test]
fn parses_mullvad_style_config() {
    let text = format!(
        "# Mullvad se-got-wg-001\n[Interface]\nPrivateKey = {PRIVATE}\n\
         Address = 10.64.12.34/32,fc00:bbbb:bbbb:bb01::1:abcd/128\nDNS = 10.64.0.1, mullvad.lan\n\n\
         [Peer]\nPublicKey = {PUBLIC}\nAllowedIPs = 0.0.0.0/0,::0/0\n\
         Endpoint = 185.213.154.68:51820\nPersistentKeepalive = 25\n\
         [Peer]\npublickey = {PUBLIC}\nEndpoint = relay.example.net:51820 # comment\n"
    );
    let config = WgQuickConfig::parse(&text).unwrap();
    let interface = &config.interface;
    assert_eq!(interface.addresses.len(), 2);
    assert_eq!(interface.addresses[1].prefix_len, 128);
    assert_eq!(interface.dns, vec![std::net::IpAddr::from([10, 64, 0, 1])]);
    assert_eq!(interface.search_domains, vec!["mullvad.lan".to_owned()]);
    assert_eq!(format!("{:?}", interface.private_key), "<redacted>");
    assert!(!format!("{config:?}").contains(PRIVATE));
    let peer = &config.peers[0];
    assert_eq!(peer.public_key.to_string(), PUBLIC);
    assert_eq!(peer.allowed_ips.len(), 2);
    assert_eq!(
        peer.endpoint,
        Some(Endpoint::Addr("185.213.154.68:51820".parse().unwrap()))
    );
    assert_eq!(peer.persistent_keepalive, Some(25));
    assert_eq!(
        config.peers[1].endpoint,
        Some(Endpoint::Host {
            host: "relay.example.net".to_owned(),
            port: 51820
        })
    );
}

#[test]
fn malformed_wgquick_errors_are_precise() {
    let interface = format!("[Interface]\nPrivateKey = {PRIVATE}\n");
    let peer = format!("[Peer]\nPublicKey = {PUBLIC}\n");
    let cases: [(String, usize, WgQuickErrorKind); 7] = [
        (
            format!("[Interface]\nAddress = 10.0.0.2/32\n{peer}"),
            1,
            WgQuickErrorKind::MissingKey("PrivateKey"),
        ),
        (
            format!("{interface}[Peer]\nAllowedIPs = 0.0.0.0/0\n"),
            3,
            WgQuickErrorKind::MissingKey("PublicKey"),
        ),
        (
            format!("{interface}[Peer]\nPublicKey = AAAA\n"),
            4,
            WgQuickErrorKind::InvalidKey("PublicKey"),
        ),
        (
            format!("[Interface]\nPrivateKey = {}\n{peer}", &PRIVATE[1..]),
            2,
            WgQuickErrorKind::InvalidKey("PrivateKey"),
        ),
        (
            format!("{interface}{peer}AllowedIPs = 10.1.2.3/8\n"),
            5,
            WgQuickErrorKind::InvalidPrefix {
                value: "10.1.2.3/8".to_owned(),
                reason: kith_types::PrefixError::HostBitsSet,
            },
        ),
        (
            format!("{interface}PostUp = iptables -A\n{peer}"),
            3,
            WgQuickErrorKind::UnsupportedKey("PostUp".to_owned()),
        ),
        (interface, 0, WgQuickErrorKind::MissingPeer),
    ];
    for (text, line, kind) in cases {
        let error = WgQuickConfig::parse(&text).unwrap_err();
        assert_eq!(error, WgQuickError { line, kind }, "{text}");
        assert!(!error.to_string().contains(&PRIVATE[1..]));
    }
}
