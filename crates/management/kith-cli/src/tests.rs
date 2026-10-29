use kith_ipc::{DnsRecord, Request, Status};
#[cfg(unix)]
use kith_ipc::{IpcServer, NetworkSummary, PathKind, PeerInfo, Response, Warning};
use kith_types::{NetworkId, NodeId};

use crate::cli::{Cmd, opts};
use crate::duration::parse_secs;
use crate::exec::{self, Names};

const NET: NetworkId = NetworkId::from_bytes([0xaa; 32]);
const NODE: NodeId = NodeId::from_bytes([0xbb; 32]);

fn names() -> Names {
    Names {
        networks: vec![(NET, "home".to_owned())],
        peers: vec![(NODE, "nas".to_owned(), "NAS".to_owned())],
        hostname: "laptop".to_owned(),
    }
}

fn parse(args: &[&str]) -> Result<Cmd, String> {
    opts()
        .run_inner(args)
        .map(|o| o.cmd)
        .map_err(|e| e.unwrap_stderr())
}

#[test]
#[expect(clippy::too_many_lines, reason = "one table row per command")]
fn every_command_parses_into_the_right_request() {
    let wg = std::env::temp_dir().join(format!("kith-cli-{}.conf", std::process::id()));
    let conf = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 10.0.0.0/24\n";
    std::fs::write(&wg, conf).unwrap();
    let wg_path = wg.to_str().unwrap();
    let id = NET.to_string();
    let cases: Vec<(Vec<&str>, Result<Request, ()>)> = vec![
        (vec!["status"], Ok(Request::Status)),
        (vec!["status", "--json"], Ok(Request::Status)),
        (
            vec!["create", "work"],
            Ok(Request::CreateNetwork {
                name: "work".into(),
                device_name: "laptop".into(),
            }),
        ),
        (
            vec!["create", "work", "--device-name", "pc"],
            Ok(Request::CreateNetwork {
                name: "work".into(),
                device_name: "pc".into(),
            }),
        ),
        (vec!["create"], Err(())),
        (
            vec!["invite", "home"],
            Ok(Request::CreateInvite {
                network: NET,
                expires_in_secs: 86_400,
                max_uses: Some(1),
            }),
        ),
        (
            vec!["invite", &id, "--expires", "7d", "--uses", "unlimited"],
            Ok(Request::CreateInvite {
                network: NET,
                expires_in_secs: 604_800,
                max_uses: None,
            }),
        ),
        (vec!["invite", "home", "--uses", "0"], Err(())),
        (vec!["invite", "home", "--expires", "soon"], Err(())),
        (vec!["invite", "nowhere"], Err(())),
        (
            vec!["join", "kith://x", "--device-name", "pc"],
            Ok(Request::Join {
                invite_link: "kith://x".into(),
                device_name: "pc".into(),
            }),
        ),
        (vec!["leave", "home"], Ok(Request::Leave { network: NET })),
        (
            vec!["remove", "home", "nas"],
            Ok(Request::RemoveDevice {
                network: NET,
                node: NODE,
            }),
        ),
        (vec!["remove", "home", "ghost"], Err(())),
        (
            vec!["peers", "home", "--json"],
            Ok(Request::Peers { network: NET }),
        ),
        (
            vec!["dns", "set", "home", "nas", "A", "100.64.0.9"],
            Ok(Request::SetRecord {
                network: NET,
                name: "nas".into(),
                record: DnsRecord::A([100, 64, 0, 9].into()),
            }),
        ),
        (
            vec!["dns", "set", "home", "nas", "aaaa", "100.64.0.9"],
            Err(()),
        ),
        (vec!["dns", "set", "home", "nas", "mx", "x"], Err(())),
        (
            vec!["dns", "rm", "home", "nas"],
            Ok(Request::DeleteRecord {
                network: NET,
                name: "nas".into(),
            }),
        ),
        (
            vec!["exit", "use", "home", "nas"],
            Ok(Request::UseExit {
                network: NET,
                node: Some(NODE),
            }),
        ),
        (vec!["exit", "use", "home", "ghost"], Err(())),
        (
            vec!["exit", "allow", "home", "nas"],
            Ok(Request::SetExitGrant {
                network: NET,
                node: NODE,
                allowed: true,
            }),
        ),
        (
            vec![
                "relay",
                "advertise",
                "--node",
                "nas",
                "home",
                "main",
                "vps.example.com:41642",
            ],
            Ok(Request::AdvertiseRelay {
                network: NET,
                name: "main".into(),
                address: "vps.example.com:41642".into(),
                node: Some(NODE),
            }),
        ),
        (
            vec!["exit", "off"],
            Ok(Request::UseExit {
                network: NET,
                node: None,
            }),
        ),
        (
            vec!["server", "--relay"],
            Ok(Request::ServerMode {
                exit: false,
                relay: true,
            }),
        ),
        (
            vec!["wg", "import", "office", wg_path],
            Ok(Request::ImportWireGuard {
                name: "office".into(),
                conf_text: conf.into(),
            }),
        ),
        (
            vec!["wg", "import", "office", "/nonexistent/kith.conf"],
            Err(()),
        ),
        (
            vec!["wg", "export", "home", "router"],
            Ok(Request::ExportWireGuard {
                network: NET,
                device_name: "router".into(),
                exit: false,
            }),
        ),
        (
            vec!["wg", "export", "--exit", "home", "phone"],
            Ok(Request::ExportWireGuard {
                network: NET,
                device_name: "phone".into(),
                exit: true,
            }),
        ),
        (vec!["frobnicate"], Err(())),
    ];
    for (args, want) in cases {
        let got = parse(&args).and_then(|cmd| exec::to_request(&cmd, &names()).map_err(|e| e.0));
        assert_eq!(got.clone().ok(), want.ok(), "{args:?}: {got:?}");
    }
    for local in [
        &["watch"][..],
        &["config", "check"],
        &["config", "check", "/x.toml"],
        &["config", "show"],
        &["daemon", "--config", "/x.toml", "--foreground", "-vv"],
    ] {
        let cmd = parse(local).unwrap();
        assert!(exec::to_request(&cmd, &names()).is_err(), "{local:?}");
    }
    std::fs::remove_file(wg).unwrap();
}

#[test]
fn durations() {
    for (text, want) in [
        ("30m", Some(1800)),
        ("24h", Some(86_400)),
        ("7d", Some(604_800)),
        ("1h30m", Some(5400)),
        ("", None),
        ("0h", None),
        ("24", None),
        ("h", None),
        ("1x", None),
        ("-1h", None),
        ("1h 30m", None),
        ("99999999999999999999d", None),
        ("30600000000000w", None),
        ("18446744073709551615s1s", None),
    ] {
        assert_eq!(parse_secs(text.to_owned()).ok(), want, "{text:?}");
    }
}

async fn run_against(path: &std::path::Path, command: &[&str]) -> String {
    let mut args = vec!["--socket", path.to_str().unwrap()];
    args.extend(command);
    let opts = opts().run_inner(&args[..]).unwrap();
    let mut out = Vec::new();
    exec::run(&opts, &mut out).await.unwrap();
    String::from_utf8(out).unwrap()
}

#[cfg(unix)]
const PHONE: NodeId = NodeId::from_bytes([0xcc; 32]);

/// What the fake daemon reports: a relay using `nas` as its exit, beside another VPN.
#[cfg(unix)]
fn fake_status() -> Status {
    Status {
        node: Some(NodeId::from_bytes([0x12; 32])),
        networks: vec![NetworkSummary {
            id: NET,
            name: "home".into(),
            peers_online: 1,
            peers_total: 2,
            relays: 1,
        }],
        exit: false,
        relay: true,
        exit_node: Some("home/nas".into()),
        listen_port: 41641,
        dns_upstreams: vec!["192.168.1.1:53".into()],
        drops: [("policy".to_owned(), 2)].into(),
        route_conflicts: vec!["100.100.100.100/32 via utun37".into()],
        warnings: vec![Warning {
            secs_ago: 30,
            message: "control channel: connection refused".into(),
        }],
    }
}

#[cfg(unix)]
fn fake_peers() -> Vec<PeerInfo> {
    let phone = PHONE;
    vec![
        PeerInfo {
            node: NODE,
            name: "nas".into(),
            address: "100.64.0.2".into(),
            path: Some(PathKind::Direct),
            path_reason: Some("direct to 198.51.100.7:41641 (12 ms)".into()),
            last_seen_secs: Some(4),
            endpoint: Some("198.51.100.7:41641".into()),
            rx_bytes: 1_234_567,
            tx_bytes: 512,
            online: true,
            display_name: "NAS".into(),
        },
        PeerInfo {
            node: phone,
            name: "phone".into(),
            address: "100.64.0.3".into(),
            path: None,
            path_reason: None,
            last_seen_secs: None,
            endpoint: None,
            rx_bytes: 0,
            tx_bytes: 0,
            online: false,
            display_name: "Phone \"old\"".into(),
        },
    ]
}

#[cfg(unix)]
#[tokio::test]
async fn status_renders_what_the_daemon_reports() {
    let path = std::env::temp_dir().join(format!("kith-cli-{}.sock", std::process::id()));
    let server = IpcServer::bind(&path).unwrap();
    let phone = PHONE;
    tokio::spawn(async move {
        loop {
            let mut conn = server.accept().await.unwrap();
            while let Some(request) = conn.next_request().await.unwrap() {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "the fake answers only what status asks"
                )]
                let response = match request {
                    Request::Status => Response::Status(fake_status()),
                    Request::Peers { network: NET } => Response::Peers(fake_peers()),
                    _ => Response::Left,
                };
                conn.respond(response).await.unwrap();
            }
        }
    });

    assert_eq!(
        run_against(&path, &["status"]).await,
        "device 12121212 (relay)\n\
         internet traffic goes through home/nas\n\
         \n\
         network home (aaaaaaaa)  1/2 online\n  \
         NAME   ADDRESS     PATH     ENDPOINT            LAST SEEN  RX       TX     NODE\n  \
         nas    100.64.0.2  direct   198.51.100.7:41641  4s ago     1.1 MiB  512 B  bbbbbbbb\n  \
         phone  100.64.0.3  offline  -                   never      0 B      0 B    cccccccc\n\
         \n\
         notes (run `kith doctor` for fixes):\n  \
         100.100.100.100/32 via utun37 overlaps kith's addresses (another VPN?)\n  \
         2 packets dropped: policy\n  \
         30s ago: control channel: connection refused\n"
    );
    assert_eq!(
        run_against(&path, &["status", "--json"]).await,
        format!(
            concat!(
                r#"{{"node":"{me}","exit":false,"relay":true,"listen_port":41641,"#,
                r#""exit_node":"home/nas","dns_upstreams":["192.168.1.1:53"],"#,
                r#""route_conflicts":["100.100.100.100/32 via utun37"],"drops":{{"policy":2}},"#,
                r#""warnings":[{{"secs_ago":30,"message":"control channel: connection refused"}}],"#,
                r#""networks":[{{"id":"{net}","#,
                r#""name":"home","peers_online":1,"peers_total":2,"relays":1,"peers":["#,
                r#"{{"node":"{nas}","name":"nas","address":"100.64.0.2","path":"direct","#,
                r#""path_reason":"direct to 198.51.100.7:41641 (12 ms)","#,
                r#""endpoint":"198.51.100.7:41641","last_seen_secs":4,"#,
                r#""rx_bytes":1234567,"tx_bytes":512,"online":true,"display_name":"NAS"}},"#,
                r#"{{"node":"{phone}","name":"phone","address":"100.64.0.3","#,
                r#""path":null,"path_reason":null,"#,
                r#""endpoint":null,"last_seen_secs":null,"#,
                r#""rx_bytes":0,"tx_bytes":0,"online":false,"display_name":"Phone \"old\""}}]}}]}}"#,
                "\n",
            ),
            me = NodeId::from_bytes([0x12; 32]),
            net = NET,
            nas = NODE,
            phone = phone,
        )
    );
    assert_eq!(
        run_against(&path, &["doctor"]).await,
        "ok    daemon is running on port 41641\n\
         ok    home/nas: direct to 198.51.100.7:41641 (12 ms)\n\
         warn  home/phone is offline: it has never connected\n      \
         fix: make sure kith runs on it (a relay is available, so a firewall is not what stops it); \
         if it is gone for good, `kith remove home phone`\n\
         ok    exit node home/nas is reachable\n\
         warn  100.100.100.100/32 via utun37 overlaps kith's addresses; kith peers there are unreachable\n      \
         fix: quit the other VPN (often Tailscale) while using kith\n\
         info  2 packets arrived unasked and the firewall refused them (policy)\n\
         warn  30s ago: control channel: connection refused\n"
    );
}

#[cfg(unix)]
#[test]
fn doctor_fails_on_a_lost_exit_and_explains_a_daemon_that_cannot_start() {
    use crate::doctor::{Level, diagnose, offline};

    let mut status = fake_status();
    status.exit_node = Some("home/phone".into());
    let lost = diagnose(&status, &[fake_peers()]);
    assert!(
        lost.iter()
            .any(|f| f.level == Level::Fail && f.what.contains("nowhere to go")),
        "{lost:?}"
    );

    let holder = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    let socket = std::path::Path::new("/run/kith/kith.sock");
    let down = offline(socket, &std::io::ErrorKind::NotFound.into(), port);
    let levels: Vec<Level> = down.iter().map(|f| f.level).collect();
    assert_eq!(levels, [Level::Fail, Level::Fail]);
    assert!(down[1].what.contains(&format!("port {port}")), "{down:?}");

    let denied = offline(socket, &std::io::ErrorKind::PermissionDenied.into(), port);
    assert_eq!(denied.len(), 1);
    assert!(
        denied[0]
            .fix
            .as_deref()
            .is_some_and(|fix| fix.contains("sudo"))
    );
}

#[test]
fn peers_are_found_by_label_or_by_a_name_only_one_of_them_asked_for() {
    let a = NodeId::from_bytes([1; 32]);
    let b = NodeId::from_bytes([2; 32]);
    let c = NodeId::from_bytes([3; 32]);
    let names = Names {
        peers: vec![
            (a, "mac".to_owned(), "Mac".to_owned()),
            (b, "mac-0202".to_owned(), "Mac".to_owned()),
            (c, "nas".to_owned(), "NAS Box".to_owned()),
        ],
        ..Names::default()
    };
    assert_eq!(names.node("mac").unwrap(), a);
    assert_eq!(names.node("mac-0202").unwrap(), b);
    assert_eq!(names.node("NAS Box").unwrap(), c);
    assert_eq!(names.node(&b.to_string()).unwrap(), b);
    let shared = names.node("Mac").unwrap_err().to_string();
    assert!(shared.contains("mac, mac-0202"), "{shared}");
    assert!(names.node("tv").is_err());
}

#[cfg(unix)]
#[test]
fn liveness_alone_decides_offline_and_the_hint_knows_about_relays() {
    use crate::doctor::{Level, diagnose};

    let mut status = fake_status();
    status.exit_node = None;
    status.networks[0].relays = 0;
    let mut peers = fake_peers();
    peers[1].last_seen_secs = Some(400);
    peers[1].path = Some(PathKind::Relay);
    let found = diagnose(&status, &[peers.clone()]);
    let phone = found
        .iter()
        .find(|f| f.what.starts_with("home/phone"))
        .unwrap();
    assert_eq!(phone.level, Level::Warn);
    assert_eq!(
        phone.what,
        "home/phone is offline: it has not been seen for 6 min"
    );
    assert!(
        phone
            .fix
            .as_deref()
            .is_some_and(|fix| fix.contains("add a relay")),
        "{phone:?}"
    );

    let mut out = Vec::new();
    crate::render::status(&mut out, &status, &[peers.clone()]).unwrap();
    let table = String::from_utf8(out).unwrap();
    let row = table
        .lines()
        .find(|l| l.trim_start().starts_with("phone"))
        .unwrap();
    assert!(row.contains("offline") && !row.contains("relay"), "{row}");

    peers[1].online = true;
    peers[1].path = None;
    peers[1].path_reason = Some("waiting for a handshake".into());
    let found = diagnose(&status, &[peers]);
    assert!(
        found.iter().any(|f| f.level == Level::Info
            && f.what == "home/phone is connecting: waiting for a handshake"),
        "{found:?}"
    );
}
