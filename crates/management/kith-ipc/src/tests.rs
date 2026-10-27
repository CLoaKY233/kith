#[cfg(unix)]
use std::collections::BTreeMap;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::{AtomicU32, Ordering};

use kith_types::{NetworkId, NodeId};
use proptest::prelude::*;
#[cfg(unix)]
use tokio::io::AsyncWriteExt;

use crate::codec::{decode, encode};
use crate::message::ServerMessage;
use crate::{
    DnsRecord, ErrorCode, Event, MAX_FRAME_LEN, NetworkSummary, PathKind, PeerInfo, Request,
    Response, Status, Warning,
};
#[cfg(unix)]
use crate::{IpcClient, IpcError, IpcServer, PROTOCOL_VERSION};

fn unframe(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = kith_wire::FrameDecoder::new(MAX_FRAME_LEN);
    decoder.push(bytes);
    decoder.next_frame().unwrap().unwrap()
}

#[test]
fn golden_bytes() {
    // A protocol contract with older CLIs: changing these bytes is a break.
    let request = Request::Peers {
        network: NetworkId::from_bytes([0xab; 32]),
    };
    let mut expected = vec![37, b'k', b't', 0x70, 4, 6];
    expected.extend([0xab; 32]);
    assert_eq!(encode(&request).unwrap(), expected);

    let response = ServerMessage::Response(Response::Error {
        code: ErrorCode::NotFound,
        message: "no".to_owned(),
    });
    assert_eq!(
        encode(&response).unwrap(),
        [10, b'k', b't', 0x70, 4, 0, 18, 3, 2, b'n', b'o']
    );

    let peers = ServerMessage::Response(Response::Peers(vec![PeerInfo {
        node: NodeId::from_bytes([0x11; 32]),
        name: "a".to_owned(),
        address: "b".to_owned(),
        path: Some(PathKind::Direct),
        path_reason: None,
        last_handshake_secs: Some(300),
        endpoint: Some("c".to_owned()),
        rx_bytes: 1,
        tx_bytes: 128,
    }]));
    let mut expected = vec![55, b'k', b't', 0x70, 4, 0, 6, 1];
    expected.extend([0x11; 32]);
    expected.extend([
        1, b'a', 1, b'b', 1, 0, 0, 1, 0xac, 0x02, 1, 1, b'c', 1, 0x80, 0x01,
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
        (net(), text()).prop_map(|(network, device_name)| Request::ExportWireGuard {
            network,
            device_name
        }),
        Just(Request::Watch),
    ]
}

fn status() -> impl Strategy<Value = Status> {
    (
        proptest::option::of(node()),
        proptest::collection::vec(
            (net(), text(), any::<u32>(), any::<u32>()).prop_map(
                |(id, name, peers_online, peers_total)| NetworkSummary {
                    id,
                    name,
                    peers_online,
                    peers_total,
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
        )
            .prop_map(
                |(
                    (node, name, address, path),
                    (last_handshake_secs, endpoint, rx_bytes, tx_bytes),
                )| {
                    PeerInfo {
                        node,
                        name,
                        address,
                        path,
                        path_reason: None,
                        last_handshake_secs,
                        endpoint,
                        rx_bytes,
                        tx_bytes,
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
        net().prop_map(|network| Response::Joined { network }),
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
        text().prop_map(|conf_text| Response::WireGuardConfig { conf_text }),
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
    let mut framed = crate::codec::Framed::new(raw);
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
