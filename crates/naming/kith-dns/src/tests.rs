use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use kith_naming::{DomainName, SplitDns, UpstreamGroupId, Zone, ZoneBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::Instant;

use crate::{DnsServer, DnsState};

const TIMEOUT: Duration = Duration::from_millis(300);
const LOOPBACK: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut message = id.to_be_bytes().to_vec();
    message.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        message.push(u8::try_from(label.len()).unwrap());
        message.extend_from_slice(label.as_bytes());
    }
    message.extend_from_slice(&[0, 0, 1, 0, 1]);
    message
}

fn query_with_edns(id: u16, name: &str, udp_payload: u16) -> Vec<u8> {
    let mut message = query(id, name);
    message[11] = 1;
    message.extend_from_slice(&[0, 0, 41]);
    message.extend_from_slice(&udp_payload.to_be_bytes());
    message.extend_from_slice(&[0; 6]);
    message
}

/// A reply to `query` with one A record holding `marker`.
fn reply(query: &[u8], marker: [u8; 4], truncated: bool) -> Vec<u8> {
    let mut message = query.to_vec();
    message[2] = 0x81 | if truncated { 0x02 } else { 0 };
    message[3] = 0x80;
    if truncated {
        return message;
    }
    message[7] = 1;
    message.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
    message.extend_from_slice(&marker);
    message
}

fn last_address(response: &[u8]) -> [u8; 4] {
    response[response.len() - 4..].try_into().unwrap()
}

fn spawn_upstream(socket: UdpSocket, marker: [u8; 4], truncated: bool) -> SocketAddr {
    let address = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buffer = [0; 512];
        loop {
            let (len, from) = socket.recv_from(&mut buffer).await.unwrap();
            let response = reply(&buffer[..len], marker, truncated);
            socket.send_to(&response, from).await.unwrap();
        }
    });
    address
}

async fn upstream(marker: [u8; 4]) -> SocketAddr {
    spawn_upstream(UdpSocket::bind(LOOPBACK).await.unwrap(), marker, false)
}

async fn truncating_upstream(size: usize) -> SocketAddr {
    let udp = UdpSocket::bind(LOOPBACK).await.unwrap();
    let tcp = TcpListener::bind(udp.local_addr().unwrap()).await.unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = tcp.accept().await.unwrap();
            let len = stream.read_u16().await.unwrap();
            let mut message = vec![0; usize::from(len)];
            stream.read_exact(&mut message).await.unwrap();
            let mut response = reply(&message, [5, 5, 5, 5], false);
            response.resize(size, 0);
            stream
                .write_all(&u16::try_from(size).unwrap().to_be_bytes())
                .await
                .unwrap();
            stream.write_all(&response).await.unwrap();
        }
    });
    spawn_upstream(udp, [0; 4], true)
}

async fn start(state: DnsState) -> SocketAddr {
    let server = DnsServer::bind_with_timeout(LOOPBACK, Arc::new(state), TIMEOUT)
        .await
        .unwrap();
    let address = server.local_addr().unwrap();
    tokio::spawn(server.run());
    address
}

fn state() -> DnsState {
    let mut zone = ZoneBuilder::new();
    zone.add_suffix(DomainName::new("home").unwrap()).unwrap();
    zone.add_device("nas", &[IpAddr::V4(Ipv4Addr::new(100, 64, 0, 7))])
        .unwrap();
    DnsState::new(zone.build().unwrap())
}

async fn ask_udp(server: SocketAddr, message: &[u8]) -> Vec<u8> {
    let client = UdpSocket::bind(LOOPBACK).await.unwrap();
    client.send_to(message, server).await.unwrap();
    let mut buffer = vec![0; 4096];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), client.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    buffer.truncate(len);
    buffer
}

async fn ask_tcp(server: SocketAddr, message: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(server).await.unwrap();
    stream
        .write_all(&u16::try_from(message.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(message).await.unwrap();
    let len = stream.read_u16().await.unwrap();
    let mut response = vec![0; usize::from(len)];
    stream.read_exact(&mut response).await.unwrap();
    response
}

#[tokio::test]
async fn device_name_resolves_over_udp_and_tcp() {
    let server = start(state()).await;
    let message = query(0x1234, "nas.home");
    for response in [
        ask_udp(server, &message).await,
        ask_tcp(server, &message).await,
    ] {
        assert_eq!(&response[..2], &[0x12, 0x34]);
        assert_eq!(response[3] & 0x0F, 0, "NOERROR");
        assert_eq!(last_address(&response), [100, 64, 0, 7]);
    }
}

#[tokio::test]
async fn split_suffix_goes_to_its_group_and_client_keeps_its_id() {
    let group_a = upstream([1, 1, 1, 1]).await;
    let group_b = upstream([2, 2, 2, 2]).await;
    let default = upstream([3, 3, 3, 3]).await;
    let state = state();
    let mut split = SplitDns::new();
    split
        .add_route(DomainName::new("corp.example").unwrap(), UpstreamGroupId(1))
        .unwrap();
    split
        .add_route(DomainName::new("lab.example").unwrap(), UpstreamGroupId(2))
        .unwrap();
    state.set_split(
        split,
        BTreeMap::from([
            (UpstreamGroupId(1), vec![group_a]),
            (UpstreamGroupId(2), vec![group_b]),
        ]),
    );
    state.set_default_upstreams(vec![default]);
    let server = start(state).await;

    let split_reply = ask_udp(server, &query(0xBEEF, "git.corp.example")).await;
    assert_eq!(&split_reply[..2], &[0xBE, 0xEF]);
    assert_eq!(last_address(&split_reply), [1, 1, 1, 1]);

    let default_reply = ask_tcp(server, &query(0x0042, "example.org")).await;
    assert_eq!(&default_reply[..2], &[0x00, 0x42]);
    assert_eq!(last_address(&default_reply), [3, 3, 3, 3]);
}

#[tokio::test]
async fn dead_upstream_fails_over_and_tcp_retry_respects_client_edns_size() {
    const FULL_REPLY_LEN: u16 = 1300;
    // Bound but silent, so the forwarder must time out rather than see ICMP.
    let dead = UdpSocket::bind(LOOPBACK).await.unwrap();
    let alive = upstream([4, 4, 4, 4]).await;
    let state = state();
    state.set_default_upstreams(vec![dead.local_addr().unwrap(), alive]);
    let failover_server = start(state).await;
    let started = Instant::now();
    let response = ask_udp(failover_server, &query(7, "example.org")).await;
    assert_eq!(last_address(&response), [4, 4, 4, 4]);
    assert!(started.elapsed() < TIMEOUT * 2);

    let state = DnsState::new(Zone::empty());
    state.set_default_upstreams(vec![truncating_upstream(FULL_REPLY_LEN.into()).await]);
    let server = start(state).await;

    let fits = ask_udp(
        server,
        &query_with_edns(8, "big.example.org", FULL_REPLY_LEN),
    )
    .await;
    assert_eq!(fits.len(), usize::from(FULL_REPLY_LEN));
    assert_eq!(fits[..2], [0, 8]);
    assert_eq!(fits[2] & 0x02, 0, "TC cleared");

    let too_big = ask_udp(
        server,
        &query_with_edns(9, "big.example.org", FULL_REPLY_LEN - 1),
    )
    .await;
    assert_eq!(
        too_big[2] & 0x02,
        0x02,
        "TC kept so the client retries over TCP"
    );
}

#[tokio::test]
async fn reply_from_another_port_is_ignored() {
    let honest = UdpSocket::bind(LOOPBACK).await.unwrap();
    let address = honest.local_addr().unwrap();
    tokio::spawn(async move {
        let spoofer = UdpSocket::bind(LOOPBACK).await.unwrap();
        let mut buffer = [0; 512];
        let (len, from) = honest.recv_from(&mut buffer).await.unwrap();
        // The spoofer knows the remapped ID, so only the source check can stop it.
        let forged = reply(&buffer[..len], [6, 6, 6, 6], false);
        spoofer.send_to(&forged, from).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let genuine = reply(&buffer[..len], [7, 7, 7, 7], false);
        honest.send_to(&genuine, from).await.unwrap();
    });
    let state = state();
    state.set_default_upstreams(vec![address]);
    let server = start(state).await;
    let response = ask_udp(server, &query(9, "example.org")).await;
    assert_eq!(last_address(&response), [7, 7, 7, 7]);
}
