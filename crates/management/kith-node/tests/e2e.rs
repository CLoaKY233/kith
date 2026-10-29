//! In-process nodes on loopback: no TUN device, no OS changes, nothing outside the test.

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, SocketAddr};
    use core::sync::atomic::{AtomicU32, Ordering};
    use core::time::Duration;
    use std::path::PathBuf;

    use kith_config::Config;
    use kith_identity::{BootstrapAddr, InviteToken};
    use kith_ipc::{PeerInfo, Request, Response};
    use kith_node::{Node, NodeHandle, NodeOptions, TunMode};
    use kith_sys::NoopSystem;
    use kith_types::{NetworkId, NodeId};
    use tokio::net::UdpSocket;
    use tokio::time::{sleep, timeout};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const INTERNET: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
    /// How long traffic may stall after a peer restarts; plain WireGuard timers take 15s.
    const RECONNECT_BUDGET: Duration = Duration::from_secs(8);

    /// Where a node keeps its state, so it can be stopped and started again.
    struct Disk {
        dir: PathBuf,
        port: core::cell::Cell<u16>,
        started: core::cell::Cell<bool>,
    }

    impl Disk {
        fn new(tag: &str) -> TestResult<Self> {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "kith-e2e-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir)?;
            Ok(Self {
                dir,
                port: core::cell::Cell::new(free_port()?),
                started: core::cell::Cell::new(false),
            })
        }

        /// Starts the node; another test may grab its port first, so a first start moves to
        /// a new port and a restart waits for the old one.
        async fn start(&self) -> TestResult<NodeHandle> {
            let mut last = None;
            for _ in 0..10 {
                let dir = self.dir.clone();
                let port = self.port.get();
                let started = start_node(
                    move |mut config| {
                        config.daemon.state_dir.clone_from(&dir);
                        config.network.listen_port = port;
                        config
                    },
                    |options| options,
                    false,
                )
                .await;
                match started {
                    Ok(node) => {
                        self.started.set(true);
                        return Ok(node);
                    }
                    Err(error) => last = Some(error),
                }
                if self.started.get() {
                    sleep(Duration::from_millis(200)).await;
                } else {
                    self.port.set(free_port()?);
                }
            }
            Err(last.unwrap_or_else(|| "node never started".into()))
        }
    }

    impl Drop for Disk {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.dir));
        }
    }

    /// A port free for both UDP (WireGuard) and TCP (control channel).
    fn free_port() -> TestResult<u16> {
        for _ in 0..50 {
            let udp = std::net::UdpSocket::bind(SocketAddr::new(LOCALHOST, 0))?;
            let port = udp.local_addr()?.port();
            if std::net::TcpListener::bind(SocketAddr::new(LOCALHOST, port)).is_ok() {
                return Ok(port);
            }
        }
        Err("no free port".into())
    }

    async fn start() -> TestResult<NodeHandle> {
        start_node(|config| config, |options| options, true).await
    }

    async fn start_server(
        exit: bool,
        relay: bool,
        hostname: Option<&str>,
    ) -> TestResult<NodeHandle> {
        let hostname = hostname.map(str::to_owned);
        start_node(
            move |mut config| {
                config.server.enabled = true;
                config.server.exit = exit;
                config.server.relay = relay;
                config.server.public_hostname.clone_from(&hostname);
                config
            },
            |options| options,
            true,
        )
        .await
    }

    /// Prints daemon debug logs when `KITH_TEST_LOG` is set; run with `--no-capture`.
    fn init_logs() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            if std::env::var_os("KITH_TEST_LOG").is_some() {
                tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::DEBUG)
                    .with_test_writer()
                    .init();
            }
        });
    }

    /// Starts a node; an auto-picked port can be taken by a parallel test between kith's
    /// check and its bind, so that one case is retried.
    async fn start_node(
        config: impl Fn(Config) -> Config,
        options_fn: impl Fn(NodeOptions) -> NodeOptions,
        in_memory: bool,
    ) -> TestResult<NodeHandle> {
        init_logs();
        let mut attempt = 0;
        loop {
            let mut base = Config::default();
            base.network.listen_port = 0;
            base.dns.enabled = true;
            let config = config(base);
            let auto_port = config.network.listen_port == 0;
            let mut options = NodeOptions::new(config, NoopSystem)
                .tun(TunMode::Memory)
                .listen_ip(LOCALHOST)
                .dns_listen(SocketAddr::new(LOCALHOST, 0))
                .without_ipc();
            if in_memory {
                options = options.in_memory_state();
            }
            attempt += 1;
            match Node::start(options_fn(options)).await {
                Err(error) if auto_port && attempt < 5 && addr_in_use(&error) => {}
                started => return Ok(started?),
            }
        }
    }

    /// Whether a port clash is the cause anywhere down `error`'s chain.
    fn addr_in_use(error: &(dyn std::error::Error + 'static)) -> bool {
        let mut cause = Some(error);
        while let Some(error) = cause {
            let io_clash = error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AddrInUse);
            let named_clash = matches!(
                error.downcast_ref::<kith_node::NodeError>(),
                Some(kith_node::NodeError::PortInUse { .. })
            );
            if io_clash || named_clash {
                return true;
            }
            cause = error.source();
        }
        false
    }

    async fn peer_info(node: &NodeHandle, network: NetworkId, name: &str) -> TestResult<PeerInfo> {
        for _ in 0..100 {
            if let Response::Peers(peers) = node.request(Request::Peers { network }).await
                && let Some(info) = peers.into_iter().find(|p| p.name == name)
            {
                return Ok(info);
            }
            sleep(Duration::from_millis(100)).await;
        }
        Err(format!("peer {name} never appeared").into())
    }

    async fn peer(node: &NodeHandle, network: NetworkId, name: &str) -> TestResult<Ipv4Addr> {
        Ok(peer_info(node, network, name).await?.address.parse()?)
    }

    /// A UDP packet from port 4000 to port 5000.
    fn udp_packet(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> TestResult<Vec<u8>> {
        udp_ports(source, 4000, destination, 5000, payload)
    }

    /// The answer to a [`udp_packet`]: ports swapped, so stateful filters see a reply.
    fn udp_reply(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> TestResult<Vec<u8>> {
        udp_ports(source, 5000, destination, 4000, payload)
    }

    fn udp_ports(
        source: Ipv4Addr,
        source_port: u16,
        destination: Ipv4Addr,
        destination_port: u16,
        payload: &[u8],
    ) -> TestResult<Vec<u8>> {
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
        packet.extend_from_slice(&source_port.to_be_bytes());
        packet.extend_from_slice(&destination_port.to_be_bytes());
        packet.extend_from_slice(&udp_len.to_be_bytes());
        packet.extend_from_slice(&[0, 0]);
        packet.extend_from_slice(payload);
        Ok(packet)
    }

    /// Resends `packet` until it comes out of `to`'s TUN; sessions may still be forming.
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

    /// Like [`deliver`], but fails if it takes longer than [`RECONNECT_BUDGET`].
    async fn reconnects(from: &NodeHandle, to: &NodeHandle, packet: &[u8]) -> TestResult {
        timeout(RECONNECT_BUDGET, deliver(from, to, packet))
            .await
            .map_err(|_| format!("no traffic within {RECONNECT_BUDGET:?} of a restart"))?
    }

    /// Sends `packet` a few times and fails if it ever comes out of `to`'s TUN.
    async fn never_delivered(from: &NodeHandle, to: &NodeHandle, packet: &[u8]) -> TestResult {
        for _ in 0..5 {
            from.send_packet(packet.to_vec()).await?;
            if let Ok(received) = timeout(Duration::from_millis(300), to.recv_packet()).await
                && received? == packet
            {
                return Err("packet arrived but should have been dropped".into());
            }
        }
        Ok(())
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

    async fn create(node: &NodeHandle, device_name: &str) -> TestResult<NetworkId> {
        let created = node
            .request(Request::CreateNetwork {
                name: "home".into(),
                device_name: device_name.into(),
            })
            .await;
        let Response::NetworkCreated { network } = created else {
            return Err(format!("create failed: {created:?}").into());
        };
        Ok(network)
    }

    async fn invite(sponsor: &NodeHandle, network: NetworkId) -> TestResult<String> {
        let response = sponsor
            .request(Request::CreateInvite {
                network,
                expires_in_secs: 600,
                max_uses: Some(1),
            })
            .await;
        let Response::Invite { invite_link } = response else {
            return Err(format!("invite failed: {response:?}").into());
        };
        Ok(invite_link)
    }

    async fn join(
        sponsor: &NodeHandle,
        joiner: &NodeHandle,
        network: NetworkId,
        device_name: &str,
    ) -> TestResult<String> {
        join_with(
            joiner,
            invite(sponsor, network).await?,
            network,
            device_name,
        )
        .await
    }

    async fn join_with(
        joiner: &NodeHandle,
        invite_link: String,
        network: NetworkId,
        device_name: &str,
    ) -> TestResult<String> {
        let outcome = joiner
            .request(Request::Join {
                invite_link,
                device_name: device_name.into(),
            })
            .await;
        if let Response::Joined {
            network: id,
            device,
        } = &outcome
            && *id == network
        {
            return Ok(device.clone());
        }
        Err(format!("join failed: {outcome:?}").into())
    }

    /// Selects `exit` once the ledger granting it has reached `node`.
    async fn use_exit(node: &NodeHandle, network: NetworkId, exit: Option<NodeId>) -> TestResult {
        let mut last = None;
        for _ in 0..100 {
            let response = node
                .request(Request::UseExit {
                    network,
                    node: exit,
                })
                .await;
            if response == Response::ExitSet {
                return Ok(());
            }
            last = Some(response);
            sleep(Duration::from_millis(100)).await;
        }
        Err(format!("exit not set: {last:?}").into())
    }

    async fn drops(node: &NodeHandle) -> TestResult<std::collections::BTreeMap<String, u64>> {
        let response = node.request(Request::Status).await;
        if let Response::Status(status) = response {
            return Ok(status.drops);
        }
        Err(format!("status failed: {response:?}").into())
    }

    async fn shutdown(nodes: impl IntoIterator<Item = NodeHandle>) -> TestResult {
        for node in nodes {
            node.shutdown().await?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn three_nodes_join_exchange_packets_and_resolve_names() -> TestResult {
        let a = start().await?;
        let b = start().await?;
        let c = start().await?;
        let network = create(&a, "alpha").await?;
        join(&a, &b, network, "beta").await?;
        if invite(&b, network).await.is_ok() {
            return Err("a non-admin member issued an invite that could never be redeemed".into());
        }
        join(&a, &c, network, "gamma").await?;
        let alpha = peer(&c, network, "alpha").await?;
        let beta = peer(&c, network, "beta").await?;
        let gamma = peer(&a, network, "gamma").await?;

        deliver(&a, &c, &udp_packet(alpha, gamma, b"a to c")?)
            .await
            .map_err(|e| format!("alpha to gamma: {e}"))?;
        deliver(&c, &b, &udp_packet(gamma, beta, b"c to b")?)
            .await
            .map_err(|e| format!("gamma to beta: {e}"))?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"b to a")?)
            .await
            .map_err(|e| format!("beta to alpha: {e}"))?;

        let Response::Status(status) = a.request(Request::Status).await else {
            return Err("status failed".into());
        };
        if status.listen_port == 0 || status.networks.first().map(|n| n.peers_total) != Some(2) {
            return Err(format!("status is wrong: {status:?}").into());
        }
        if online_counts_agree(&a, network).await? != 2 {
            return Err("both peers should be online".into());
        }
        let reason = peer_info(&a, network, "gamma").await?.path_reason;
        if !reason
            .as_deref()
            .is_some_and(|r| r.starts_with("direct to 127.0.0.1:"))
        {
            return Err(format!("gamma's path reason is {reason:?}").into());
        }

        // A device reaches its own address without the packet leaving it, as macOS needs.
        deliver(&a, &a, &udp_packet(alpha, alpha, b"to myself")?)
            .await
            .map_err(|e| format!("alpha to itself: {e}"))?;

        let dns = c.dns_addr().ok_or("no DNS server")?;
        for (name, expected) in [("alpha", alpha), ("beta", beta), ("gamma", gamma)] {
            let resolved = resolve(dns, &format!("{name}.home.kith.internal")).await?;
            if resolved != expected {
                return Err(format!("{name} resolved to {resolved}, expected {expected}").into());
            }
        }
        let own = resolve(
            a.dns_addr().ok_or("no DNS server")?,
            "alpha.home.kith.internal",
        )
        .await?;
        if own != alpha {
            return Err(format!("alpha resolved its own name to {own}").into());
        }
        shutdown([a, b, c]).await
    }

    /// Status's online count and the peer list come from one rule, so they always agree.
    async fn online_counts_agree(node: &NodeHandle, network: NetworkId) -> TestResult<u32> {
        let Response::Status(status) = node.request(Request::Status).await else {
            return Err("status failed".into());
        };
        let Response::Peers(peers) = node.request(Request::Peers { network }).await else {
            return Err("peers failed".into());
        };
        let listed = peers.iter().filter(|p| p.online).count();
        let counted = status
            .networks
            .iter()
            .find(|n| n.id == network)
            .map(|n| n.peers_online);
        if counted != Some(u32::try_from(listed)?) {
            return Err(format!("status says {counted:?} online, peers list {listed}").into());
        }
        if let Some(p) = peers
            .iter()
            .find(|p| !p.online && (p.path.is_some() || p.endpoint.is_some()))
        {
            return Err(format!("{} is offline but shows a path: {p:?}", p.name).into());
        }
        Ok(status
            .networks
            .iter()
            .find(|n| n.id == network)
            .map_or(0, |n| n.peers_online))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn devices_asking_for_one_name_each_get_their_own() -> TestResult {
        let a = start().await?;
        let b = start().await?;
        let c = start().await?;
        let network = create(&a, "alpha").await?;
        let first = join(&a, &b, network, "Work Mac").await?;
        let second = join(&a, &c, network, "Work Mac").await?;
        if first != "work-mac" || !second.starts_with("work-mac-") || second == first {
            return Err(format!("labels are {first} and {second}").into());
        }
        let b_addr = peer(&a, network, &first).await?;
        let c_addr = peer(&a, network, &second).await?;
        for node in [&a, &b, &c] {
            let dns = node.dns_addr().ok_or("no DNS server")?;
            for (label, expected) in [(&first, b_addr), (&second, c_addr)] {
                let got = resolve(dns, &format!("{label}.home.kith.internal")).await?;
                if got != expected {
                    return Err(format!("{label} resolved to {got}, expected {expected}").into());
                }
            }
        }
        let info = peer_info(&a, network, &second).await?;
        if info.display_name != "Work Mac" {
            return Err(format!("the asked-for name is lost: {info:?}").into());
        }
        if online_counts_agree(&a, network).await? != 2 {
            return Err("both peers should be online".into());
        }

        let b_node = peer_info(&a, network, &first).await?.node;
        if remove(&a, network, b_node).await != Response::DeviceRemoved {
            return Err("removal failed".into());
        }
        let dns = a.dns_addr().ok_or("no DNS server")?;
        let mut promoted = false;
        for _ in 0..100 {
            if peer_info(&a, network, &first)
                .await
                .is_ok_and(|p| p.node == info.node)
                && resolve(dns, &format!("{first}.home.kith.internal"))
                    .await
                    .ok()
                    == Some(c_addr)
            {
                promoted = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        if !promoted {
            return Err("the remaining device did not take the plain name".into());
        }
        online_counts_agree(&a, network).await?;
        shutdown([a, b, c]).await
    }

    async fn remove(by: &NodeHandle, network: NetworkId, node: NodeId) -> Response {
        by.request(Request::RemoveDevice { network, node }).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_admin_removes_a_device_and_it_is_cut_off() -> TestResult {
        let a = start().await?;
        let b = start().await?;
        let c = start().await?;
        let network = create(&a, "alpha").await?;
        join(&a, &b, network, "beta").await?;
        join(&a, &c, network, "gamma").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta_info = peer_info(&a, network, "beta").await?;
        let beta: Ipv4Addr = beta_info.address.parse()?;
        let me = peer_info(&b, network, "alpha").await?.node;
        deliver(&b, &a, &udp_packet(beta, alpha, b"before removal")?).await?;

        if remove(&a, network, me).await == Response::DeviceRemoved {
            return Err("an admin removed itself instead of being told to leave".into());
        }
        let gamma = peer_info(&b, network, "gamma").await?.node;
        if remove(&b, network, gamma).await == Response::DeviceRemoved {
            return Err("a non-admin removed a device".into());
        }

        let removed = remove(&a, network, beta_info.node).await;
        if removed != Response::DeviceRemoved {
            return Err(format!("removal failed: {removed:?}").into());
        }
        for (node, name) in [(&a, "alpha"), (&c, "gamma")] {
            let mut gone = false;
            for _ in 0..100 {
                let Response::Peers(peers) = node.request(Request::Peers { network }).await else {
                    return Err("peers failed".into());
                };
                if peers.iter().all(|p| p.name != "beta") {
                    gone = true;
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
            if !gone {
                return Err(format!("{name} still lists beta after its removal").into());
            }
        }
        never_delivered(&b, &a, &udp_packet(beta, alpha, b"after removal")?)
            .await
            .map_err(|e| format!("a removed device still reached alpha: {e}"))?;
        shutdown([a, b, c]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restarted_nodes_reconnect_from_their_saved_state() -> TestResult {
        let disk_a = Disk::new("a")?;
        let disk_b = Disk::new("b")?;
        let a = disk_a.start().await?;
        let b = disk_b.start().await?;
        let network = create(&a, "alpha").await?;
        join(&a, &b, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta = peer(&a, network, "beta").await?;
        deliver(&a, &b, &udp_packet(alpha, beta, b"before")?).await?;

        b.shutdown().await?;
        let b = disk_b.start().await?;
        reconnects(&a, &b, &udp_packet(alpha, beta, b"b restarted")?).await?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"b restarted")?).await?;

        shutdown([a, b]).await?;
        let a = disk_a.start().await?;
        let b = disk_b.start().await?;
        if peer(&b, network, "alpha").await? != alpha {
            return Err("alpha's address changed across a restart".into());
        }
        reconnects(&a, &b, &udp_packet(alpha, beta, b"both restarted")?).await?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"both restarted")?).await?;
        shutdown([a, b]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_server_set_up_by_config_relays_for_peers_without_direct_udp() -> TestResult {
        let s = start_server(false, true, None).await?;
        let a = start_node(|config| config, NodeOptions::without_direct_udp, true).await?;
        let b = start_node(|config| config, NodeOptions::without_direct_udp, true).await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        join(&s, &b, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta = peer(&a, network, "beta").await?;

        deliver(&a, &b, &udp_packet(alpha, beta, b"via relay")?).await?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"via relay")?).await?;

        // The relay's own node is reachable through its relay too, as a lone server is.
        let server = peer(&a, network, "server").await?;
        deliver(&a, &s, &udp_packet(alpha, server, b"to the relay host")?)
            .await
            .map_err(|e| format!("alpha to the relay host: {e}"))?;
        deliver(&s, &a, &udp_packet(server, alpha, b"from the relay host")?)
            .await
            .map_err(|e| format!("relay host to alpha: {e}"))?;
        let beta_info = peer_info(&a, network, "beta").await?;
        if beta_info.endpoint.as_deref() != Some("relay")
            || !beta_info
                .path_reason
                .as_deref()
                .is_some_and(|r| r.starts_with("relay: "))
        {
            return Err(format!("beta is not shown as relayed: {beta_info:?}").into());
        }
        shutdown([a, b, s]).await
    }

    /// Waits until `node` reaches `name` over a path matching `want`.
    async fn path_becomes(
        node: &NodeHandle,
        network: NetworkId,
        name: &str,
        want: impl Fn(&str) -> bool,
    ) -> TestResult<String> {
        let mut last = None;
        for _ in 0..300 {
            last = peer_info(node, network, name).await?.endpoint;
            if last.as_deref().is_some_and(&want) {
                return Ok(last.unwrap_or_default());
            }
            sleep(Duration::from_millis(100)).await;
        }
        Err(format!("{name} stayed on {last:?}").into())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn traffic_falls_back_to_the_relay_and_returns_to_direct() -> TestResult {
        let s = start_server(false, true, None).await?;
        let a = start().await?;
        let b = start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        join(&s, &b, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta = peer(&a, network, "beta").await?;
        let direct = |path: &str| path != "relay";

        deliver(&a, &b, &udp_packet(alpha, beta, b"direct")?).await?;
        path_becomes(&a, network, "beta", direct).await?;

        a.block_direct_udp(true).await?;
        b.block_direct_udp(true).await?;
        deliver(&a, &b, &udp_packet(alpha, beta, b"relayed")?).await?;
        deliver(&b, &a, &udp_packet(beta, alpha, b"relayed")?).await?;
        path_becomes(&a, network, "beta", |path| path == "relay").await?;

        a.block_direct_udp(false).await?;
        b.block_direct_udp(false).await?;
        path_becomes(&a, network, "beta", direct).await?;
        path_becomes(&b, network, "alpha", direct).await?;
        deliver(&a, &b, &udp_packet(alpha, beta, b"direct again")?).await?;
        shutdown([a, b, s]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exit_traffic_goes_out_and_back_and_the_choice_survives_a_restart() -> TestResult {
        let s = start_server(true, false, None).await?;
        let disk = Disk::new("exit")?;
        let a = disk.start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        let alpha = peer(&s, network, "alpha").await?;
        let server = peer_info(&a, network, "server").await?.node;

        let outbound = udp_packet(alpha, INTERNET, b"to the internet")?;
        let reply = udp_reply(INTERNET, alpha, b"from the internet")?;
        never_delivered(&a, &s, &outbound)
            .await
            .map_err(|e| format!("before any exit: {e}"))?;

        use_exit(&a, network, Some(server)).await?;
        let stale = udp_packet(
            Ipv4Addr::new(192, 168, 1, 5),
            INTERNET,
            b"opened before the exit",
        )?;
        never_delivered(&a, &s, &stale)
            .await
            .map_err(|e| format!("traffic from a LAN address reached the exit: {e}"))?;
        if !drops(&a).await?.contains_key("foreign_source") {
            return Err("the client did not count traffic from a foreign source".into());
        }
        if drops(&s).await?.contains_key("source_not_allowed") {
            return Err("foreign-source traffic still reached the exit node".into());
        }
        deliver(&a, &s, &outbound)
            .await
            .map_err(|e| format!("exit on, out: {e}"))?;
        deliver(&s, &a, &reply)
            .await
            .map_err(|e| format!("exit on, back: {e}"))?;

        a.shutdown().await?;
        let a = disk.start().await?;
        deliver(&a, &s, &outbound)
            .await
            .map_err(|e| format!("after restart, out: {e}"))?;
        deliver(&s, &a, &reply)
            .await
            .map_err(|e| format!("after restart, back: {e}"))?;

        use_exit(&a, network, None).await?;
        let after_off = udp_packet(alpha, INTERNET, b"after exit off")?;
        never_delivered(&a, &s, &after_off)
            .await
            .map_err(|e| format!("exit off: {e}"))?;
        a.shutdown().await?;
        let a = disk.start().await?;
        let after_restart = udp_packet(alpha, INTERNET, b"off after restart")?;
        never_delivered(&a, &s, &after_restart)
            .await
            .map_err(|e| format!("exit off after restart: {e}"))?;
        shutdown([a, s]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_taken_port_fails_with_an_error_that_names_it() -> TestResult {
        let port = free_port()?;
        let _holder = std::net::UdpSocket::bind(SocketAddr::new(LOCALHOST, port))?;
        let started = start_node(
            |mut config| {
                config.network.listen_port = port;
                config
            },
            |options| options,
            true,
        )
        .await;
        let Err(error) = started else {
            return Err("started on a port another program holds".into());
        };
        let message = error.to_string();
        if !message.contains(&format!("port {port}")) || !message.contains("listen_port") {
            return Err(format!("unhelpful error: {message}").into());
        }
        if !addr_in_use(&*error) {
            return Err(format!("a port clash is not recognised as one: {error:?}").into());
        }
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invites_lead_with_the_public_hostname_and_joining_through_it_works() -> TestResult {
        let s = start_server(false, true, Some("localhost")).await?;
        let a = start().await?;
        let network = create(&s, "server").await?;
        let link = invite(&s, network).await?;

        let token = InviteToken::parse(&link)?;
        let first = token.bootstrap().first().ok_or("no bootstrap entries")?;
        let BootstrapAddr::Host(host, _) = first.addr() else {
            return Err(format!("first bootstrap entry is {first}, not the hostname").into());
        };
        if host.as_str() != "localhost" {
            return Err(format!("bootstrap host is {host}, expected localhost").into());
        }

        join_with(&a, link, network, "alpha").await?;
        peer(&a, network, "server").await?;
        shutdown([a, s]).await
    }

    async fn export(
        node: &NodeHandle,
        network: NetworkId,
        name: &str,
        exit: bool,
    ) -> TestResult<String> {
        let response = node
            .request(Request::ExportWireGuard {
                network,
                device_name: name.into(),
                exit,
            })
            .await;
        let Response::WireGuardConfig { conf_text } = response else {
            return Err(format!("export failed: {response:?}").into());
        };
        Ok(conf_text)
    }

    fn conf_value<'a>(conf: &'a str, key: &str) -> TestResult<&'a str> {
        conf.lines()
            .find_map(|line| line.strip_prefix(key)?.trim_start().strip_prefix('='))
            .map(str::trim)
            .ok_or_else(|| format!("no {key} in {conf}").into())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_phone_with_plain_wireguard_reaches_the_server_and_its_exit() -> TestResult {
        let s = start_server(true, false, Some("localhost")).await?;
        let a = start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        let server: Ipv4Addr = peer(&a, network, "server").await?;

        let mesh_only = export(&s, network, "phone", false).await?;
        if !conf_value(&mesh_only, "Endpoint")?.starts_with("localhost:") {
            return Err(format!("the export skips the public hostname: {mesh_only}").into());
        }
        if conf_value(&mesh_only, "AllowedIPs")?.contains("0.0.0.0/0") {
            return Err(format!("a mesh-only export takes all traffic: {mesh_only}").into());
        }
        if export(&a, network, "phone", true).await.is_ok() {
            return Err("a device that is not an exit exported an exit config".into());
        }
        let full = export(&s, network, "phone", true).await?;
        if conf_value(&full, "AllowedIPs")? != "0.0.0.0/0, ::/0" {
            return Err(format!("an exit export leaves traffic outside: {full}").into());
        }
        let phone: Ipv4Addr = conf_value(&full, "Address")?
            .trim_end_matches("/32")
            .parse()?;

        let p = start().await?;
        let imported = p
            .request(Request::ImportWireGuard {
                name: "phone".into(),
                conf_text: full.replace("Endpoint = localhost:", "Endpoint = 127.0.0.1:"),
            })
            .await;
        if !matches!(imported, Response::Imported { .. }) {
            return Err(format!("the phone config did not import: {imported:?}").into());
        }
        deliver(&p, &s, &udp_packet(phone, server, b"phone to server")?)
            .await
            .map_err(|e| format!("phone to server: {e}"))?;
        deliver(&s, &p, &udp_packet(server, phone, b"server to phone")?)
            .await
            .map_err(|e| format!("server to phone: {e}"))?;
        deliver(
            &p,
            &s,
            &udp_packet(phone, INTERNET, b"phone to the internet")?,
        )
        .await
        .map_err(|e| format!("phone to the internet: {e}"))?;
        shutdown([p, a, s]).await
    }
}
