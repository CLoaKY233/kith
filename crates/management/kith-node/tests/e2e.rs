//! Two in-process nodes: create, invite, join, exchange packets and resolve names.

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, SocketAddr};
    use core::time::Duration;

    use kith_config::Config;
    use kith_ipc::{PeerInfo, Request, Response};
    use kith_node::{Node, NodeHandle, NodeOptions, TunMode};
    use kith_sys::NoopSystem;
    use kith_types::NetworkId;
    use tokio::net::UdpSocket;
    use tokio::time::{sleep, timeout};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    async fn start() -> TestResult<NodeHandle> {
        let mut config = Config::default();
        config.network.listen_port = 0;
        config.dns.enabled = true;
        let options = NodeOptions::new(config, NoopSystem)
            .tun(TunMode::Memory)
            .in_memory_state()
            .listen_ip(LOCALHOST)
            .dns_listen(SocketAddr::new(LOCALHOST, 0))
            .without_ipc();
        Ok(Node::start(options).await?)
    }

    async fn peer(node: &NodeHandle, network: NetworkId, name: &str) -> TestResult<Ipv4Addr> {
        for _ in 0..100 {
            if let Response::Peers(peers) = node.request(Request::Peers { network }).await
                && let Some(PeerInfo { address, .. }) = peers.into_iter().find(|p| p.name == name)
            {
                return Ok(address.parse()?);
            }
            sleep(Duration::from_millis(100)).await;
        }
        Err(format!("peer {name} never appeared").into())
    }

    fn udp_packet(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> TestResult<Vec<u8>> {
        let udp_len = u16::try_from(8 + payload.len())?;
        let [t0, t1] = (udp_len + 20).to_be_bytes();
        let [s0, s1, s2, s3] = source.octets();
        let [d0, d1, d2, d3] = destination.octets();
        let mut header = [
            0x45, 0, t0, t1, 0, 0, 0x40, 0, 64, 17, 0, 0, s0, s1, s2, s3, d0, d1, d2, d3,
        ];
        let mut sum: u32 = header
            .as_chunks::<2>()
            .0
            .iter()
            .map(|word| u32::from(u16::from_be_bytes(*word)))
            .sum();
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        let [c0, c1] = (!u16::try_from(sum)?).to_be_bytes();
        header[10] = c0;
        header[11] = c1;
        let mut packet = header.to_vec();
        packet.extend_from_slice(&4000u16.to_be_bytes());
        packet.extend_from_slice(&5000u16.to_be_bytes());
        packet.extend_from_slice(&udp_len.to_be_bytes());
        packet.extend_from_slice(&[0, 0]);
        packet.extend_from_slice(payload);
        Ok(packet)
    }

    async fn deliver(from: &NodeHandle, to: &NodeHandle, packet: &[u8]) -> TestResult {
        for _ in 0..150 {
            from.send_packet(packet.to_vec()).await?;
            if let Ok(received) = timeout(Duration::from_millis(200), to.recv_packet()).await
                && received? == packet
            {
                return Ok(());
            }
        }
        Err("packet never arrived".into())
    }

    async fn resolve(dns: SocketAddr, name: &str) -> TestResult<Ipv4Addr> {
        let mut query = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            query.push(u8::try_from(label.len())?);
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        let socket = UdpSocket::bind(SocketAddr::new(LOCALHOST, 0)).await?;
        socket.send_to(&query, dns).await?;
        let mut buffer = [0u8; 512];
        let (len, _) = timeout(Duration::from_secs(5), socket.recv_from(&mut buffer)).await??;
        let answer = buffer.get(..len).ok_or("short response")?;
        let Some(&[_, _, _, _, _, _, 0, 1, ..]) = answer.get(..12) else {
            return Err(format!("not one answer for {name}").into());
        };
        let Some(&[0, 4, a, b, c, d]) = answer.get(len.saturating_sub(6)..) else {
            return Err(format!("no A record for {name}").into());
        };
        Ok(Ipv4Addr::new(a, b, c, d))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn two_nodes_join_exchange_packets_and_resolve_names() -> TestResult {
        let a = start().await?;
        let b = start().await?;
        let Response::NetworkCreated { network } = a
            .request(Request::CreateNetwork {
                name: "home".into(),
                device_name: "alpha".into(),
            })
            .await
        else {
            return Err("create failed".into());
        };
        let Response::Invite { invite_link } = a
            .request(Request::CreateInvite {
                network,
                expires_in_secs: 600,
                max_uses: Some(1),
            })
            .await
        else {
            return Err("invite failed".into());
        };
        let joined = b
            .request(Request::Join {
                invite_link,
                device_name: "beta".into(),
            })
            .await;
        if joined != (Response::Joined { network }) {
            return Err(format!("join failed: {joined:?}").into());
        }
        let beta = peer(&a, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;

        deliver(&a, &b, &udp_packet(alpha, beta, b"ping")?).await?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"pong")?).await?;

        let dns = a.dns_addr().ok_or("no DNS server")?;
        let resolved = resolve(dns, "beta.home.kith.internal").await?;
        if resolved != beta {
            return Err(format!("resolved {resolved}, expected {beta}").into());
        }
        a.shutdown().await?;
        b.shutdown().await?;
        Ok(())
    }
}
