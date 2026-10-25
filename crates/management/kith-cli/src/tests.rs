use kith_ipc::{
    DnsRecord, IpcServer, NetworkSummary, PathKind, PeerInfo, Request, Response, Status,
};
use kith_types::{NetworkId, NodeId};

use crate::cli::{Cmd, opts};
use crate::duration::parse_secs;
use crate::exec::{self, Names};

const NET: NetworkId = NetworkId::from_bytes([0xaa; 32]);
const NODE: NodeId = NodeId::from_bytes([0xbb; 32]);

fn names() -> Names {
    Names {
        networks: vec![(NET, "home".to_owned())],
        peers: vec![(NODE, "nas".to_owned())],
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
        &["daemon", "--config", "/x.toml", "--foreground"],
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

#[tokio::test]
async fn status_renders_what_the_daemon_reports() {
    let path = std::env::temp_dir().join(format!("kith-cli-{}.sock", std::process::id()));
    let server = IpcServer::bind(&path).unwrap();
    tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        while let Some(request) = conn.next_request().await.unwrap() {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "the fake answers only what status asks"
            )]
            let response = match request {
                Request::Status => Response::Status(Status {
                    node: Some(NodeId::from_bytes([0x12; 32])),
                    networks: vec![NetworkSummary {
                        id: NET,
                        name: "home".into(),
                        peers_online: 1,
                        peers_total: 2,
                    }],
                    exit: false,
                    relay: true,
                }),
                Request::Peers { network: NET } => Response::Peers(vec![
                    PeerInfo {
                        node: NODE,
                        name: "nas".into(),
                        address: "100.64.0.2".into(),
                        path: Some(PathKind::Relay),
                    },
                    PeerInfo {
                        node: NodeId::from_bytes([0xcc; 32]),
                        name: "phone".into(),
                        address: "100.64.0.3".into(),
                        path: None,
                    },
                ]),
                _ => Response::Left,
            };
            conn.respond(response).await.unwrap();
        }
    });
    let args = ["--socket", path.to_str().unwrap(), "status"];
    let opts = opts().run_inner(&args[..]).unwrap();
    let mut out = Vec::new();
    exec::run(&opts, &mut out).await.unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "device 12121212 (relay)\n\
         \n\
         network home (aaaaaaaa)  1/2 online\n  \
         NAME   ADDRESS     PATH     NODE\n  \
         nas    100.64.0.2  relay    bbbbbbbb\n  \
         phone  100.64.0.3  offline  cccccccc\n"
    );
}
