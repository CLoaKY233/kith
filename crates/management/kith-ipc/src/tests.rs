#[cfg(unix)]
use std::collections::BTreeMap;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::{AtomicU32, Ordering};

use kith_types::{NetworkId, NodeId};
use proptest::prelude::*;
#[cfg(unix)]
use tokio::io::AsyncWriteExt;

use crate::codec::{Framed, decode, encode};
use crate::message::ServerMessage;
use crate::{
    BlockedInfo, DnsRecord, Entry, EntryKind, ErrorCode, Event, ImportedInfo, InviteInfo, IpcError,
    LinkInfo, LinkReport, MAX_FRAME_LEN, MAX_RESPONSE_LEN, NetworkSummary, PROTOCOL_VERSION,
    PathKind, PeerInfo, PendingJoin, PolicyInfo, PolicyRuleInfo, Problem, RecordInfo, RelayInfo,
    Request, Response, SelectorInfo, Status, TargetInfo, TrafficSample, TrafficSeries, Warning,
};
#[cfg(unix)]
use crate::{IpcClient, IpcServer};

fn unframe(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = kith_wire::FrameDecoder::new(MAX_RESPONSE_LEN);
    decoder.push(bytes);
    decoder.next_frame().unwrap().unwrap()
}

#[test]
fn golden_bytes() {
    // A protocol contract with older CLIs: changing these bytes is a break.
    let request = Request::Peers {
        network: NetworkId::from_bytes([0xab; 32]),
    };
    let mut expected = vec![37, b'k', b't', 0x70, 14, 6];
    expected.extend([0xab; 32]);
    assert_eq!(encode(&request).unwrap(), expected);

    let response = ServerMessage::Response(Response::Error {
        code: ErrorCode::NotFound,
        message: "no".to_owned(),
    });
    assert_eq!(
        encode(&response).unwrap(),
        [10, b'k', b't', 0x70, 14, 0, 18, 3, 2, b'n', b'o']
    );

    let peers = ServerMessage::Response(Response::Peers(vec![PeerInfo {
        node: NodeId::from_bytes([0x11; 32]),
        name: "a".to_owned(),
        address: "b".to_owned(),
        path: Some(PathKind::Direct),
        path_reason: None,
        last_seen_secs: Some(300),
        endpoint: Some("c".to_owned()),
        rx_bytes: 1,
        tx_bytes: 128,
        online: true,
        display_name: "D".to_owned(),
        gateway: Some("e".to_owned()),
        path_mtu: Some(1216),
    }]));
    let mut expected = vec![64, b'k', b't', 0x70, 14, 0, 6, 1];
    expected.extend([0x11; 32]);
    expected.extend([
        1, b'a', 1, b'b', 1, 0, 0, 1, 0xac, 0x02, 1, 1, b'c', 1, 0x80, 0x01, 1, 1, b'D', 1, 1,
        b'e', 1, 0xc0, 0x09,
    ]);
    assert_eq!(encode(&peers).unwrap(), expected);
}

fn text() -> impl Strategy<Value = String> {
    "[a-z0-9.= ]{0,12}"
}
fn node() -> impl Strategy<Value = NodeId> {
    any::<[u8; 32]>().prop_map(NodeId::from_bytes)
}
fn net() -> impl Strategy<Value = NetworkId> {
    any::<[u8; 32]>().prop_map(NetworkId::from_bytes)
}
fn path() -> impl Strategy<Value = PathKind> {
    prop_oneof![
        Just(PathKind::Direct),
        Just(PathKind::Relay),
        Just(PathKind::Stealth)
    ]
}

fn request() -> impl Strategy<Value = Request> {
    let record = prop_oneof![
        any::<[u8; 4]>().prop_map(|b| DnsRecord::A(b.into())),
        any::<[u8; 16]>().prop_map(|b| DnsRecord::Aaaa(b.into())),
        text().prop_map(DnsRecord::Cname),
        text().prop_map(DnsRecord::Txt),
    ];
    prop_oneof![
        (any::<u32>(), text()).prop_map(|(protocol, client_version)| Request::Hello {
            protocol,
            client_version
        }),
        Just(Request::Status),
        (text(), text())
            .prop_map(|(name, device_name)| Request::CreateNetwork { name, device_name }),
        (net(), any::<u64>(), any::<Option<u32>>()).prop_map(
            |(network, expires_in_secs, max_uses)| Request::CreateInvite {
                network,
                expires_in_secs,
                max_uses
            }
        ),
        (text(), text()).prop_map(|(invite_link, device_name)| Request::Join {
            invite_link,
            device_name
        }),
        net().prop_map(|network| Request::Leave { network }),
        net().prop_map(|network| Request::Peers { network }),
        (net(), text(), record).prop_map(|(network, name, record)| Request::SetRecord {
            network,
            name,
            record
        }),
        (net(), text()).prop_map(|(network, name)| Request::DeleteRecord { network, name }),
        (net(), text()).prop_map(|(network, policy_toml)| Request::SetPolicy {
            network,
            policy_toml
        }),
        (net(), proptest::option::of(node()))
            .prop_map(|(network, node)| Request::UseExit { network, node }),
        (net(), text(), text(), proptest::option::of(node())).prop_map(
            |(network, name, address, node)| Request::AdvertiseRelay {
                network,
                name,
                address,
                node
            }
        ),
        (net(), text()).prop_map(|(network, name)| Request::RemoveRelay { network, name }),
        (net(), node(), any::<bool>()).prop_map(|(network, node, allowed)| {
            Request::SetExitGrant {
                network,
                node,
                allowed,
            }
        }),
        (any::<bool>(), any::<bool>())
            .prop_map(|(exit, relay)| Request::ServerMode { exit, relay }),
        (text(), text()).prop_map(|(name, conf_text)| Request::ImportWireGuard { name, conf_text }),
        (net(), text(), proptest::collection::vec(text(), 0..3)).prop_map(
            |(network, device_name, tags)| Request::ExportWireGuard {
                network,
                device_name,
                tags,
            }
        ),
        Just(Request::Watch),
    ]
}

fn status() -> impl Strategy<Value = Status> {
    (
        proptest::option::of(node()),
        proptest::collection::vec(
            (
                (net(), text(), any::<u32>(), any::<u32>(), any::<u32>()),
                any::<bool>(),
                proptest::collection::vec(
                    (text(), text()).prop_map(|(what, fix)| Problem { what, fix }),
                    0..2,
                ),
            )
                .prop_map(
                    |((id, name, peers_online, peers_total, relays), admin, problems)| {
                        NetworkSummary {
                            id,
                            name,
                            peers_online,
                            peers_total,
                            relays,
                            problems,
                            admin,
                        }
                    },
                ),
            0..3,
        ),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(node, networks, exit, relay)| Status {
            node,
            networks,
            exit,
            relay,
            exit_node: exit.then(|| "home/vps".to_owned()),
            listen_port: 41641,
            dns_upstreams: vec!["192.0.2.53:53".to_owned()],
            drops: [("policy".to_owned(), 3)].into(),
            route_conflicts: Vec::new(),
            warnings: vec![Warning {
                secs_ago: 5,
                message: "w".to_owned(),
            }],
            imported: Vec::new(),
        })
}

fn server_message() -> impl Strategy<Value = ServerMessage> {
    let status = status();
    let peers = proptest::collection::vec(
        (
            (node(), text(), text(), proptest::option::of(path())),
            (
                any::<Option<u64>>(),
                proptest::option::of(text()),
                any::<u64>(),
                any::<u64>(),
            ),
            (
                any::<bool>(),
                text(),
                proptest::option::of(text()),
                any::<Option<u16>>(),
            ),
        )
            .prop_map(
                |(
                    (node, name, address, path),
                    (last_seen_secs, endpoint, rx_bytes, tx_bytes),
                    (online, display_name, gateway, path_mtu),
                )| {
                    PeerInfo {
                        node,
                        name,
                        address,
                        path,
                        path_reason: None,
                        last_seen_secs,
                        endpoint,
                        rx_bytes,
                        tx_bytes,
                        online,
                        display_name,
                        gateway,
                        path_mtu,
                    }
                },
            ),
        0..3,
    );
    let code = prop_oneof![
        Just(ErrorCode::VersionMismatch),
        Just(ErrorCode::HelloRequired),
        Just(ErrorCode::PermissionDenied),
        Just(ErrorCode::NotFound),
        Just(ErrorCode::InvalidArgument),
        Just(ErrorCode::Internal),
    ];
    let response = prop_oneof![
        (any::<u32>(), text()).prop_map(|(protocol, daemon_version)| Response::Hello {
            protocol,
            daemon_version
        }),
        status.prop_map(Response::Status),
        net().prop_map(|network| Response::NetworkCreated { network }),
        text().prop_map(|invite_link| Response::Invite { invite_link }),
        (net(), text()).prop_map(|(network, device)| Response::Joined { network, device }),
        Just(Response::Left),
        peers.prop_map(Response::Peers),
        Just(Response::RecordSet),
        Just(Response::RecordDeleted),
        Just(Response::PolicySet),
        Just(Response::ExitSet),
        text().prop_map(|address| Response::RelayAdvertised { address }),
        Just(Response::RelayRemoved),
        Just(Response::ExitGrantSet),
        proptest::collection::vec(text(), 0..3).prop_map(|notes| Response::ServerModeSet { notes }),
        net().prop_map(|network| Response::Imported { network }),
        (text(), text(), text(), proptest::option::of(text())).prop_map(
            |(device, endpoint, mesh, everything)| Response::WireGuardConfig {
                device,
                endpoint,
                mesh,
                everything,
            }
        ),
        Just(Response::Watching),
        (code, text()).prop_map(|(code, message)| Response::Error { code, message }),
    ];
    let event = prop_oneof![
        (net(), node(), path()).prop_map(|(network, node, path)| Event::PeerConnected {
            network,
            node,
            path
        }),
        (net(), node()).prop_map(|(network, node)| Event::PeerDisconnected { network, node }),
        net().prop_map(|network| Event::NetworkChanged { network }),
        text().prop_map(|message| Event::Warning { message }),
    ];
    prop_oneof![
        response.prop_map(ServerMessage::Response),
        event.prop_map(ServerMessage::Event),
    ]
}

proptest! {
    #[test]
    fn every_message_round_trips(request in request(), reply in server_message()) {
        prop_assert_eq!(decode::<Request>(&unframe(&encode(&request).unwrap())).unwrap(), request);
        prop_assert_eq!(decode::<ServerMessage>(&unframe(&encode(&reply).unwrap())).unwrap(), reply);
    }

    #[test]
    fn arbitrary_bytes_never_panic(input in proptest::collection::vec(any::<u8>(), 0..128)) {
        let _request = decode::<Request>(&input);
        let _reply = decode::<ServerMessage>(&input);
    }
}

#[cfg(unix)]
fn socket_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kith-ipc-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("s")
}

/// Serves one connection: answers `Status`, and streams two events on `Watch`.
#[cfg(unix)]
async fn serve_one(server: IpcServer) -> Option<u32> {
    let mut conn = server.accept().await.unwrap();
    let uid = conn.peer().map(|peer| peer.uid);
    while let Ok(Some(request)) = conn.next_request().await {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "the test server answers everything else with Status"
        )]
        match request {
            Request::Watch => {
                conn.respond(Response::Watching).await.unwrap();
                for message in ["a", "b"] {
                    let event = Event::Warning {
                        message: message.to_owned(),
                    };
                    conn.send_event(event).await.unwrap();
                }
                break;
            }
            _ => conn
                .respond(Response::Status(Status {
                    node: None,
                    networks: Vec::new(),
                    exit: false,
                    relay: true,
                    exit_node: None,
                    listen_port: 41641,
                    dns_upstreams: Vec::new(),
                    drops: BTreeMap::new(),
                    route_conflicts: Vec::new(),
                    warnings: Vec::new(),
                    imported: Vec::new(),
                }))
                .await
                .unwrap(),
        }
    }
    uid
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn request_watch_and_peer_cred_over_a_real_socket() {
    let path = socket_path();
    // A stale socket file left by a crashed daemon is replaced.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    let server = tokio::spawn(serve_one(IpcServer::bind(&path).unwrap()));

    let mut client = IpcClient::connect(&path).await.unwrap();
    let Response::Status(status) = client.request(Request::Status).await.unwrap() else {
        panic!("expected status");
    };
    assert!(status.relay);

    let mut events = client.watch().await.unwrap();
    for expected in ["a", "b"] {
        let event = events.recv().await.unwrap().unwrap();
        assert_eq!(
            event,
            Event::Warning {
                message: expected.to_owned()
            }
        );
    }
    assert!(events.recv().await.is_none());

    let our_uid = std::fs::metadata(path.parent().unwrap())
        .map(|meta| std::os::unix::fs::MetadataExt::uid(&meta))
        .unwrap();
    assert_eq!(server.await.unwrap(), Some(our_uid));
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn version_mismatch_is_a_clean_error() {
    let path = socket_path();
    let server = IpcServer::bind(&path).unwrap();
    let handler = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        conn.next_request().await
    });

    let mut raw = tokio::net::UnixStream::connect(&path).await.unwrap();
    let hello = Request::Hello {
        protocol: 99,
        client_version: "future".to_owned(),
    };
    raw.write_all(&encode(&hello).unwrap()).await.unwrap();
    let mut framed = Framed::new(raw, MAX_RESPONSE_LEN, MAX_FRAME_LEN);
    let reply = framed.recv::<ServerMessage>().await.unwrap();
    assert!(matches!(
        reply,
        Some(ServerMessage::Response(Response::Error {
            code: ErrorCode::VersionMismatch,
            ..
        }))
    ));
    assert!(matches!(
        handler.await.unwrap(),
        Err(IpcError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: 99
        })
    ));
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn bind_creates_a_missing_socket_directory() {
    let dir = socket_path().with_file_name("run").join("kith");
    let path = dir.join("kith.sock");
    let server = IpcServer::bind(&path).unwrap();
    assert!(dir.is_dir());
    drop(server);
}

/// An hour of traffic for busy peers, as much as the daemon keeps.
fn hour_of_traffic(peers: u8) -> Response {
    let series = (0..peers)
        .map(|peer| TrafficSeries {
            network: NetworkId::from_bytes([0xab; 32]),
            node: NodeId::from_bytes([peer; 32]),
            name: format!("peer-{peer}"),
            samples: (0..1800_u64)
                .map(|tick| TrafficSample {
                    at_ms: 1_762_000_000_000 + tick * 2000,
                    rx_bytes: 40_000_000_000 + tick * 1_000_000,
                    tx_bytes: 9_000_000_000 + tick * 300_000,
                })
                .collect(),
        })
        .collect();
    Response::Traffic(series)
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_traffic_history_reaches_the_client() {
    let (daemon, cli) = tokio::io::duplex(64 * 1024);
    let mut daemon = Framed::new(daemon, MAX_FRAME_LEN, MAX_RESPONSE_LEN);
    let mut cli = Framed::new(cli, MAX_RESPONSE_LEN, MAX_FRAME_LEN);
    let reply = ServerMessage::Response(hour_of_traffic(12));
    assert!(encode(&reply).unwrap().len() > MAX_FRAME_LEN);
    let sent = reply.clone();
    let sender = tokio::spawn(async move { daemon.send(&sent).await });
    assert_eq!(cli.recv::<ServerMessage>().await.unwrap(), Some(reply));
    sender.await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn an_oversized_request_fails_before_it_is_sent() {
    let (cli, mut daemon) = tokio::io::duplex(1024);
    let mut cli = Framed::new(cli, MAX_RESPONSE_LEN, MAX_FRAME_LEN);
    let request = Request::SetPolicy {
        network: NET,
        policy_toml: "#".repeat(MAX_FRAME_LEN),
    };
    assert!(matches!(
        cli.send(&request).await,
        Err(IpcError::Wire(kith_wire::WireError::TooLarge { .. }))
    ));
    drop(cli);
    let mut sent = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut daemon, &mut sent)
        .await
        .unwrap();
    assert_eq!(sent, Vec::<u8>::new());
}

/// The postcard variant index each request must keep while `PROTOCOL_VERSION` stays.
/// Adding a variant fails to compile here, so its sample and the fingerprint get updated.
fn request_index(request: &Request) -> u8 {
    match request {
        Request::Hello { .. } => 0,
        Request::Status => 1,
        Request::CreateNetwork { .. } => 2,
        Request::CreateInvite { .. } => 3,
        Request::Join { .. } => 4,
        Request::Leave { .. } => 5,
        Request::Peers { .. } => 6,
        Request::SetRecord { .. } => 7,
        Request::DeleteRecord { .. } => 8,
        Request::SetPolicy { .. } => 9,
        Request::UseExit { .. } => 10,
        Request::AdvertiseRelay { .. } => 11,
        Request::RemoveRelay { .. } => 12,
        Request::SetExitGrant { .. } => 13,
        Request::ServerMode { .. } => 14,
        Request::ImportWireGuard { .. } => 15,
        Request::ExportWireGuard { .. } => 16,
        Request::Watch => 17,
        Request::RemoveDevice { .. } => 18,
        Request::Records { .. } => 19,
        Request::Journal { .. } => 20,
        Request::Traffic => 21,
        Request::Topology { .. } => 22,
        Request::RequestCode { .. } => 23,
        Request::RequestJoin { .. } => 24,
        Request::PendingJoins => 25,
        Request::ApproveJoin { .. } => 26,
        Request::DenyJoin { .. } => 27,
        Request::Blocked { .. } => 28,
        Request::AllowBlocked { .. } => 29,
        Request::RenameDevice { .. } => 30,
        Request::ListInvites { .. } => 31,
        Request::RevokeInvite { .. } => 32,
        Request::GetPolicy { .. } => 33,
        Request::ListRelays { .. } => 34,
        Request::ListExits { .. } => 35,
        Request::UseImport { .. } => 36,
        Request::RemoveImport { .. } => 37,
    }
}

fn response_index(response: &Response) -> u8 {
    match response {
        Response::Hello { .. } => 0,
        Response::Status(_) => 1,
        Response::NetworkCreated { .. } => 2,
        Response::Invite { .. } => 3,
        Response::Joined { .. } => 4,
        Response::Left => 5,
        Response::Peers(_) => 6,
        Response::RecordSet => 7,
        Response::RecordDeleted => 8,
        Response::PolicySet => 9,
        Response::ExitSet => 10,
        Response::RelayAdvertised { .. } => 11,
        Response::RelayRemoved => 12,
        Response::ExitGrantSet => 13,
        Response::ServerModeSet { .. } => 14,
        Response::Imported { .. } => 15,
        Response::WireGuardConfig { .. } => 16,
        Response::Watching => 17,
        Response::Error { .. } => 18,
        Response::DeviceRemoved => 19,
        Response::Records(_) => 20,
        Response::Journaling { .. } => 21,
        Response::Traffic(_) => 22,
        Response::Topology(_) => 23,
        Response::RequestCode { .. } => 24,
        Response::JoinPending { .. } => 25,
        Response::JoinDenied { .. } => 26,
        Response::PendingJoins(_) => 27,
        Response::JoinApproved { .. } => 28,
        Response::JoinRequestDenied => 29,
        Response::Blocked(_) => 30,
        Response::BlockedAllowed => 31,
        Response::DeviceRenamed => 32,
        Response::Invites(_) => 33,
        Response::InviteRevoked => 34,
        Response::Policy(_) => 35,
        Response::Relays(_) => 36,
        Response::Exits(_) => 37,
        Response::ImportUsed => 38,
        Response::ImportRemoved => 39,
    }
}

fn event_index(event: &Event) -> u8 {
    match event {
        Event::PeerConnected { .. } => 0,
        Event::PeerDisconnected { .. } => 1,
        Event::NetworkChanged { .. } => 2,
        Event::Warning { .. } => 3,
    }
}

fn entry_index(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::PeerOnline { .. } => 0,
        EntryKind::PeerOffline { .. } => 1,
        EntryKind::PathChanged { .. } => 2,
        EntryKind::EndpointChanged { .. } => 3,
        EntryKind::DeviceJoined { .. } => 4,
        EntryKind::DeviceRemoved { .. } => 5,
        EntryKind::ExitChanged { .. } => 6,
        EntryKind::Dropped { .. } => 7,
        EntryKind::Warning { .. } => 8,
        EntryKind::Log { .. } => 9,
        EntryKind::JoinRequested { .. } => 10,
        EntryKind::Blocked { .. } => 11,
        EntryKind::DeviceRenamed { .. } => 12,
    }
}

const NET: NetworkId = NetworkId::from_bytes([0xab; 32]);
const NODE: NodeId = NodeId::from_bytes([0x11; 32]);

#[expect(clippy::too_many_lines, reason = "one sample per request variant")]
fn requests() -> Vec<Request> {
    let s = |text: &str| text.to_owned();
    vec![
        Request::Hello {
            protocol: 12,
            client_version: s("0.1.1"),
        },
        Request::Status,
        Request::CreateNetwork {
            name: s("home"),
            device_name: s("mac"),
        },
        Request::CreateInvite {
            network: NET,
            expires_in_secs: 3600,
            max_uses: Some(2),
        },
        Request::Join {
            invite_link: s("kith://join/x"),
            device_name: s("mac"),
        },
        Request::Leave { network: NET },
        Request::Peers { network: NET },
        Request::SetRecord {
            network: NET,
            name: s("nas"),
            record: DnsRecord::Aaaa([0xfd; 16].into()),
        },
        Request::DeleteRecord {
            network: NET,
            name: s("nas"),
        },
        Request::SetPolicy {
            network: NET,
            policy_toml: s("[[rule]]"),
        },
        Request::UseExit {
            network: NET,
            node: Some(NODE),
        },
        Request::AdvertiseRelay {
            network: NET,
            name: s("r"),
            address: s("relay:443"),
            node: None,
        },
        Request::RemoveRelay {
            network: NET,
            name: s("r"),
        },
        Request::SetExitGrant {
            network: NET,
            node: NODE,
            allowed: true,
        },
        Request::ServerMode {
            exit: true,
            relay: false,
        },
        Request::ImportWireGuard {
            name: s("wg"),
            conf_text: s("[Interface]"),
        },
        Request::ExportWireGuard {
            network: NET,
            device_name: s("phone"),
            tags: vec![s("mobile")],
        },
        Request::Watch,
        Request::RemoveDevice {
            network: NET,
            node: NODE,
        },
        Request::Records { network: NET },
        Request::Journal { since: Some(300) },
        Request::Traffic,
        Request::Topology { network: NET },
        Request::RequestCode { network: NET },
        Request::RequestJoin {
            request_code: s("kith://request/x"),
            device_name: s("tv"),
        },
        Request::PendingJoins,
        Request::ApproveJoin {
            network: NET,
            node: NODE,
        },
        Request::DenyJoin {
            network: NET,
            node: NODE,
        },
        Request::Blocked { network: NET },
        Request::AllowBlocked {
            network: NET,
            from: NODE,
            to: NodeId::from_bytes([0x22; 32]),
            protocol: s("tcp"),
            port: Some(22),
        },
        Request::RenameDevice {
            network: NET,
            node: NODE,
            name: s("nas"),
        },
        Request::ListInvites { network: NET },
        Request::RevokeInvite {
            network: NET,
            invite: s("ab"),
        },
        Request::GetPolicy { network: NET },
        Request::ListRelays { network: NET },
        Request::ListExits { network: NET },
        Request::UseImport {
            network: NET,
            active: true,
        },
        Request::RemoveImport { network: NET },
    ]
}

#[expect(clippy::too_many_lines, reason = "one sample per response variant")]
fn responses() -> Vec<Response> {
    let s = |text: &str| text.to_owned();
    vec![
        Response::Hello {
            protocol: 12,
            daemon_version: s("0.1.1"),
        },
        Response::Status(Status {
            node: Some(NODE),
            networks: vec![NetworkSummary {
                id: NET,
                name: s("home"),
                peers_online: 1,
                peers_total: 2,
                relays: 0,
                problems: vec![Problem {
                    what: s("w"),
                    fix: s("f"),
                }],
                admin: true,
            }],
            exit: false,
            relay: true,
            exit_node: Some(s("home/vps")),
            listen_port: 41641,
            dns_upstreams: vec![s("192.0.2.53:53")],
            drops: [(s("policy"), 3)].into(),
            route_conflicts: vec![s("100.64.0.0/10 on utun4")],
            warnings: vec![Warning {
                secs_ago: 5,
                message: s("w"),
            }],
            imported: vec![ImportedInfo {
                network: NET,
                name: s("mullvad"),
                endpoint: Some(s("198.51.100.1:51820")),
                routes: vec![s("0.0.0.0/0")],
                everything: true,
                active: false,
                last_seen_secs: Some(7),
                rx_bytes: 1,
                tx_bytes: 2,
            }],
        }),
        Response::NetworkCreated { network: NET },
        Response::Invite {
            invite_link: s("kith://join/x"),
        },
        Response::Joined {
            network: NET,
            device: s("mac-2"),
        },
        Response::Left,
        Response::Peers(vec![sample_peer()]),
        Response::RecordSet,
        Response::RecordDeleted,
        Response::PolicySet,
        Response::ExitSet,
        Response::RelayAdvertised {
            address: s("relay:443"),
        },
        Response::RelayRemoved,
        Response::ExitGrantSet,
        Response::ServerModeSet {
            notes: vec![s("n")],
        },
        Response::Imported { network: NET },
        Response::WireGuardConfig {
            device: s("phone"),
            endpoint: s("198.51.100.1:41641"),
            mesh: s("[Interface]"),
            everything: None,
        },
        Response::Watching,
        Response::Error {
            code: ErrorCode::PermissionDenied,
            message: s("no"),
        },
        Response::DeviceRemoved,
        Response::Records(vec![RecordInfo {
            name: s("nas.home.kith.internal"),
            record: DnsRecord::A([100, 64, 0, 1].into()),
            device: true,
        }]),
        Response::Journaling { next: 7 },
        Response::Traffic(vec![TrafficSeries {
            network: NET,
            node: NODE,
            name: s("nas"),
            samples: vec![TrafficSample {
                at_ms: 1_762_000_000_000,
                rx_bytes: 1,
                tx_bytes: 2,
            }],
        }]),
        Response::Topology(vec![LinkReport {
            node: NODE,
            name: s("nas"),
            age_secs: 0,
            links: vec![LinkInfo {
                peer: NodeId::from_bytes([0x22; 32]),
                peer_name: s("mac"),
                path: PathKind::Stealth,
                relay: Some(NODE),
                rtt_ms: Some(12),
                bytes_per_sec: 1000,
            }],
        }]),
        Response::RequestCode {
            code: s("kith://request/x"),
        },
        Response::JoinPending {
            network: NET,
            network_name: s("home"),
        },
        Response::JoinDenied { network: NET },
        Response::PendingJoins(vec![PendingJoin {
            network: NET,
            node: NODE,
            name: s("tv"),
            from: Some(s("192.0.2.9:41641")),
            age_secs: 4,
        }]),
        Response::JoinApproved { device: s("tv") },
        Response::JoinRequestDenied,
        Response::Blocked(vec![BlockedInfo {
            network: NET,
            from: NODE,
            from_name: s("tv"),
            to: NodeId::from_bytes([0x22; 32]),
            to_name: s("nas"),
            protocol: s("udp"),
            port: Some(53),
            count: 9,
            last_ms: 1_762_000_000_000,
            rule: s("[[rule]]"),
        }]),
        Response::BlockedAllowed,
        Response::DeviceRenamed,
        Response::Invites(vec![InviteInfo {
            id: s("ab"),
            expires_at_ms: 1_762_000_000_000,
            uses_left: Some(1),
            created_at_ms: 1_761_000_000_000,
        }]),
        Response::InviteRevoked,
        Response::Policy(PolicyInfo {
            toml: s("[[rules]]"),
            rules: vec![PolicyRuleInfo {
                sources: vec![SelectorInfo {
                    kind: s("any"),
                    value: None,
                    name: None,
                }],
                destinations: vec![TargetInfo {
                    selector: SelectorInfo {
                        kind: s("tag"),
                        value: Some(s("server")),
                        name: None,
                    },
                    protocol: s("tcp"),
                    ports: vec![s("22"), s("8000-8080")],
                }],
            }],
        }),
        Response::Relays(vec![RelayInfo {
            name: s("r"),
            address: s("relay:443"),
            node: NODE,
        }]),
        Response::Exits(vec![NODE]),
        Response::ImportUsed,
        Response::ImportRemoved,
    ]
}

fn sample_peer() -> PeerInfo {
    PeerInfo {
        node: NODE,
        name: "nas".to_owned(),
        address: "100.64.0.1".to_owned(),
        path: Some(PathKind::Relay),
        path_reason: Some("direct timed out".to_owned()),
        last_seen_secs: Some(3),
        endpoint: None,
        rx_bytes: 10,
        tx_bytes: 20,
        online: true,
        display_name: "nas".to_owned(),
        gateway: None,
        path_mtu: None,
    }
}

fn events() -> Vec<Event> {
    vec![
        Event::PeerConnected {
            network: NET,
            node: NODE,
            path: PathKind::Direct,
        },
        Event::PeerDisconnected {
            network: NET,
            node: NODE,
        },
        Event::NetworkChanged { network: NET },
        Event::Warning {
            message: "w".to_owned(),
        },
    ]
}

fn entry_kinds() -> Vec<EntryKind> {
    let s = |text: &str| text.to_owned();
    vec![
        EntryKind::PeerOnline {
            network: NET,
            node: NODE,
            name: s("nas"),
            path: Some(PathKind::Direct),
            gateway: None,
        },
        EntryKind::PeerOffline {
            network: NET,
            node: NODE,
            name: s("nas"),
        },
        EntryKind::PathChanged {
            network: NET,
            node: NODE,
            name: s("nas"),
            from: Some(PathKind::Direct),
            to: None,
        },
        EntryKind::EndpointChanged {
            network: NET,
            node: NODE,
            name: s("phone"),
            endpoint: s("198.51.100.2:5000"),
        },
        EntryKind::DeviceJoined {
            network: NET,
            node: NODE,
            name: s("tv"),
            address: s("100.64.0.9"),
        },
        EntryKind::DeviceRemoved {
            network: NET,
            node: NODE,
            name: s("tv"),
        },
        EntryKind::ExitChanged {
            exit: Some(s("home/vps")),
        },
        EntryKind::Dropped {
            reason: s("policy"),
            count: 3,
        },
        EntryKind::Warning { message: s("w") },
        EntryKind::Log {
            level: s("info"),
            target: s("kith_node"),
            message: s("m"),
        },
        EntryKind::JoinRequested {
            network: NET,
            node: NODE,
            name: s("tv"),
        },
        EntryKind::Blocked {
            network: NET,
            from: NODE,
            from_name: s("tv"),
            to: NodeId::from_bytes([0x22; 32]),
            to_name: s("nas"),
            protocol: s("icmp"),
            port: None,
            count: 1,
        },
        EntryKind::DeviceRenamed {
            network: NET,
            node: NODE,
            old_name: s("tv"),
            name: s("lounge"),
        },
    ]
}

/// The envelope body of a message, after the header.
fn body(bytes: &[u8]) -> Vec<u8> {
    let payload = unframe(bytes);
    let (_, body) = kith_wire::envelope::decode_header(&payload).unwrap();
    body.to_vec()
}

/// FNV-1a: enough to notice any changed byte, with no dependency.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn every_variant_keeps_its_wire_index_and_bytes() {
    // A CLI and daemon of the same PROTOCOL_VERSION must agree on every byte. If this
    // fails because a message changed, bump PROTOCOL_VERSION and update the fingerprint.
    let mut all = Vec::new();
    for (index, request) in requests().into_iter().enumerate() {
        assert_eq!(usize::from(request_index(&request)), index);
        let bytes = encode(&request).unwrap();
        let body = body(&bytes);
        assert_eq!(body.first().copied(), Some(request_index(&request)));
        assert_eq!(decode::<Request>(&unframe(&bytes)).unwrap(), request);
        all.extend(body);
    }
    let messages = responses()
        .into_iter()
        .enumerate()
        .map(|(index, response)| {
            assert_eq!(usize::from(response_index(&response)), index);
            (
                0,
                response_index(&response),
                ServerMessage::Response(response),
            )
        })
        .chain(events().into_iter().enumerate().map(|(index, event)| {
            assert_eq!(usize::from(event_index(&event)), index);
            (1, event_index(&event), ServerMessage::Event(event))
        }))
        .chain(entry_kinds().into_iter().enumerate().map(|(index, kind)| {
            assert_eq!(usize::from(entry_index(&kind)), index);
            let at = u64::try_from(index).unwrap();
            let entry = Entry {
                seq: at,
                at_ms: 1_762_000_000_000 + at,
                kind,
            };
            (2, 0, ServerMessage::Entry(entry))
        }));
    for (outer, inner, message) in messages {
        let bytes = encode(&message).unwrap();
        let body = body(&bytes);
        assert_eq!(body.first().copied(), Some(outer));
        if outer != 2 {
            assert_eq!(body.get(1).copied(), Some(inner));
        }
        assert_eq!(decode::<ServerMessage>(&unframe(&bytes)).unwrap(), message);
        all.extend(body);
    }
    assert_eq!(
        (PROTOCOL_VERSION, fingerprint(&all)),
        (14, 0x08ca_916e_b999_e4a6),
        "the wire format changed"
    );
}
