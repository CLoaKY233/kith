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

    /// A port free for both UDP (WireGuard) and TCP (control channel), below every OS's
    /// range for ports picked on their own (Linux from 32768, macOS from 49152), so a node
    /// that restarts on it never finds another test's port-0 socket there: at most
    /// 20000 + 499 * 20 + 2000 = 31980.
    fn free_port() -> TestResult<u16> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let start = 20_000 + (std::process::id() % 500) * 20;
        for _ in 0..200 {
            let step = NEXT.fetch_add(1, Ordering::Relaxed) % 2_000;
            let port = u16::try_from(start + step)?;
            let udp = std::net::UdpSocket::bind(SocketAddr::new(LOCALHOST, port));
            let tcp = std::net::TcpListener::bind(SocketAddr::new(LOCALHOST, port));
            if udp.is_ok() && tcp.is_ok() {
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

    async fn dns_upstreams(node: &NodeHandle) -> TestResult<Vec<String>> {
        let response = node.request(Request::Status).await;
        if let Response::Status(status) = response {
            return Ok(status.dns_upstreams);
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

    /// Polls `node`'s view of `name` until `done` holds, for at most `within`.
    async fn wait_for_peer(
        node: &NodeHandle,
        network: NetworkId,
        name: &str,
        within: Duration,
        done: impl Fn(&PeerInfo) -> bool,
    ) -> TestResult<PeerInfo> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let info = peer_info(node, network, name).await?;
            if done(&info) {
                return Ok(info);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("{name} never reached the expected state: {info:?}").into());
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_silent_peer_goes_offline_quickly_and_is_remembered_across_restarts() -> TestResult {
        let disk_a = Disk::new("seen")?;
        let a = disk_a.start().await?;
        let b = start().await?;
        let network = create(&a, "alpha").await?;
        join(&a, &b, network, "beta").await?;
        wait_for_peer(&a, network, "beta", Duration::from_secs(15), |p| p.online).await?;
        for _ in 0..15 {
            let idle = peer_info(&a, network, "beta").await?;
            if !idle.online || idle.last_seen_secs.is_none_or(|secs| secs >= 12) {
                return Err(format!("an idle running peer went quiet: {idle:?}").into());
            }
            sleep(Duration::from_secs(1)).await;
        }

        b.shutdown().await?;
        let gone = tokio::time::Instant::now();
        let offline =
            wait_for_peer(&a, network, "beta", Duration::from_secs(35), |p| !p.online).await?;
        if offline.path.is_some() || offline.endpoint.is_some() || offline.last_seen_secs.is_none()
        {
            return Err(format!(
                "an offline peer shows a path or lost its last-seen time: {offline:?}"
            )
            .into());
        }
        let took = gone.elapsed();

        a.shutdown().await?;
        let a = disk_a.start().await?;
        let after = peer_info(&a, network, "beta").await?;
        let silent_for = took.as_secs().saturating_sub(1);
        if after.online || after.last_seen_secs.is_none_or(|secs| secs < silent_for) {
            return Err(format!("a restart forgot when beta was last seen: {after:?}").into());
        }
        shutdown([a]).await
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

    /// How long a peer that went quiet long ago may take to move to the relay, or back:
    /// a lost direct path is distrusted after 6.5s, and pings go out every 3 to 5s.
    const FAILOVER_BUDGET: Duration = Duration::from_secs(15);

    #[tokio::test(flavor = "current_thread")]
    async fn an_idle_peer_stays_online_through_the_relay_and_comes_back_direct() -> TestResult {
        let s = start_server(false, true, None).await?;
        let a = start().await?;
        let b = start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        join(&s, &b, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta = peer(&a, network, "beta").await?;
        deliver(&a, &b, &udp_packet(alpha, beta, b"direct")?).await?;
        path_becomes(&a, network, "beta", |path| path != "relay").await?;
        sleep(Duration::from_secs(50)).await;

        a.block_direct_udp(true).await?;
        b.block_direct_udp(true).await?;
        let relayed = timeout(FAILOVER_BUDGET, async {
            loop {
                let info = peer_info(&a, network, "beta").await?;
                if info.online && info.endpoint.as_deref() == Some("relay") {
                    return Ok::<_, Box<dyn std::error::Error>>(());
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await;
        if !matches!(relayed, Ok(Ok(()))) {
            let info = peer_info(&a, network, "beta").await?;
            return Err(
                format!("a long-idle peer did not move to the relay in time: {info:?}").into(),
            );
        }
        timeout(
            FAILOVER_BUDGET,
            deliver(&a, &b, &udp_packet(alpha, beta, b"relayed")?),
        )
        .await
        .map_err(|_| "no traffic through the relay in time")??;

        a.block_direct_udp(false).await?;
        b.block_direct_udp(false).await?;
        timeout(
            FAILOVER_BUDGET,
            path_becomes(&a, network, "beta", |path| path != "relay"),
        )
        .await
        .map_err(|_| "did not come back to direct in time")??;
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
        if !dns_upstreams(&a).await?.iter().any(|u| u == "1.1.1.1:53") {
            return Err(
                "with an exit, names should resolve through it, not the local network".into(),
            );
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
        if dns_upstreams(&a).await?.iter().any(|u| u == "1.1.1.1:53") {
            return Err("with the exit off, names should resolve as the OS does".into());
        }
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

    struct Exported {
        device: String,
        mesh: String,
        everything: Option<String>,
    }

    async fn export(
        node: &NodeHandle,
        network: NetworkId,
        name: &str,
        tags: &[&str],
    ) -> TestResult<Exported> {
        let response = node
            .request(Request::ExportWireGuard {
                network,
                device_name: name.into(),
                tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            })
            .await;
        let Response::WireGuardConfig {
            device,
            mesh,
            everything,
            ..
        } = response
        else {
            return Err(format!("export failed: {response:?}").into());
        };
        Ok(Exported {
            device,
            mesh,
            everything,
        })
    }

    fn conf_value<'a>(conf: &'a str, key: &str) -> TestResult<&'a str> {
        conf.lines()
            .find_map(|line| line.strip_prefix(key)?.trim_start().strip_prefix('='))
            .map(str::trim)
            .ok_or_else(|| format!("no {key} in {conf}").into())
    }

    /// Starts a node that is nothing but a plain WireGuard device using `conf`, as a phone is.
    async fn phone_from(conf: &str) -> TestResult<NodeHandle> {
        let p = start().await?;
        let imported = p
            .request(Request::ImportWireGuard {
                name: "phone".into(),
                conf_text: conf.replace("Endpoint = localhost:", "Endpoint = 127.0.0.1:"),
            })
            .await;
        if !matches!(imported, Response::Imported { .. }) {
            return Err(format!("the phone config did not import: {imported:?}").into());
        }
        Ok(p)
    }

    fn phone_policy(server: NodeId) -> TestResult<String> {
        use kith_policy::{Policy, PortRange, PortSet, Protocol, Rule, Selector, TagName, Target};
        let rule = Rule::new(
            vec![Selector::Tag(TagName::new("phone")?)],
            vec![Target::new(
                Selector::Device(server),
                Protocol::Any,
                PortSet::ranges(vec![PortRange::new(1, 65535)?])?,
            )?],
        )?;
        Ok(toml::to_string(&Policy::new(vec![rule])?)?)
    }

    /// Checks a new phone's two configs and returns its address and the all-traffic one.
    fn checked_configs(exported: Exported) -> TestResult<(Ipv4Addr, String)> {
        if exported.device != "phone" {
            return Err(format!("the phone is called {}", exported.device).into());
        }
        if !conf_value(&exported.mesh, "Endpoint")?.starts_with("localhost:") {
            return Err(format!("the config skips the public hostname: {}", exported.mesh).into());
        }
        let address = conf_value(&exported.mesh, "Address")?;
        let phone: Ipv4Addr = address.split('/').next().unwrap_or_default().parse()?;
        if !address.contains("/128")
            || conf_value(&exported.mesh, "MTU")? != "1280"
            || conf_value(&exported.mesh, "AllowedIPs")?.contains("0.0.0.0/0")
        {
            return Err(format!("the mesh config is wrong: {}", exported.mesh).into());
        }
        let everything = exported
            .everything
            .ok_or("an exit server gave no everything config")?;
        if conf_value(&everything, "AllowedIPs")? != "0.0.0.0/0, ::/0"
            || conf_value(&everything, "PrivateKey")? != conf_value(&exported.mesh, "PrivateKey")?
        {
            return Err(format!("the everything config is wrong: {everything}").into());
        }
        Ok((phone, everything))
    }

    async fn set_policy(node: &NodeHandle, network: NetworkId, policy_toml: String) -> TestResult {
        let set = node
            .request(Request::SetPolicy {
                network,
                policy_toml,
            })
            .await;
        if set != Response::PolicySet {
            return Err(format!("policy not set: {set:?}").into());
        }
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_device_that_restarts_still_sees_phones_online() -> TestResult {
        let s = start_server(true, false, Some("localhost")).await?;
        let disk_a = Disk::new("a")?;
        let a = disk_a.start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        let server: Ipv4Addr = peer(&a, network, "server").await?;
        let (phone, everything) = checked_configs(export(&s, network, "Phone", &["phone"]).await?)?;
        let p = phone_from(&everything).await?;
        deliver(&p, &s, &udp_packet(phone, server, b"hello")?).await?;
        wait_for_peer(&a, network, "phone", Duration::from_secs(15), |p| p.online).await?;
        let on_gateway = peer_info(&s, network, "phone").await?;
        if on_gateway.path != Some(kith_ipc::PathKind::Direct) {
            return Err(
                format!("the gateway should reach its phone directly: {on_gateway:?}").into(),
            );
        }

        a.shutdown().await?;
        let a = disk_a.start().await?;
        let seen = wait_for_peer(&a, network, "phone", Duration::from_secs(15), |p| p.online)
            .await
            .map_err(|e| format!("after a restart alpha never saw the phone online: {e}"))?;
        if seen.last_seen_secs.is_none_or(|secs| secs > 30) {
            return Err(format!("the phone's last seen is stale: {seen:?}").into());
        }
        shutdown([a, p, s]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_phone_on_plain_wireguard_is_a_full_member() -> TestResult {
        let s = start_server(true, false, Some("localhost")).await?;
        let a = start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        let server: Ipv4Addr = peer(&a, network, "server").await?;
        let alpha: Ipv4Addr = peer(&s, network, "alpha").await?;
        if export(&a, network, "Phone", &[]).await.is_ok() {
            return Err("a member that is not an admin added a phone".into());
        }

        let (phone, everything) = checked_configs(export(&s, network, "Phone", &["phone"]).await?)?;

        let p = phone_from(&everything).await?;
        deliver(&p, &s, &udp_packet(phone, server, b"phone to its server")?)
            .await
            .map_err(|e| format!("phone to its server: {e}"))?;
        deliver(&p, &a, &udp_packet(phone, alpha, b"phone to alpha")?)
            .await
            .map_err(|e| format!("phone to another device: {e}"))?;
        deliver(&a, &p, &udp_packet(alpha, phone, b"alpha to phone")?)
            .await
            .map_err(|e| format!("another device to the phone: {e}"))?;
        deliver(
            &p,
            &s,
            &udp_packet(phone, INTERNET, b"phone to the internet")?,
        )
        .await
        .map_err(|e| format!("phone to the internet: {e}"))?;

        let seen =
            wait_for_peer(&a, network, "phone", Duration::from_secs(15), |p| p.online).await?;
        if seen.gateway.as_deref() != Some("server") || seen.display_name != "Phone" {
            return Err(format!("alpha sees the phone wrong: {seen:?}").into());
        }
        let dns = a.dns_addr().ok_or("no DNS server")?;
        if resolve(dns, "phone.home.kith.internal").await? != phone {
            return Err("the phone's name does not resolve".into());
        }

        let server_node = peer_info(&a, network, "server").await?.node;
        let alpha_node = peer_info(&s, network, "alpha").await?.node;
        set_policy(&s, network, phone_policy(server_node)?).await?;
        let reply = udp_reply(server, phone, b"reply from the server")?;
        deliver(&p, &s, &udp_packet(phone, server, b"allowed by policy")?).await?;
        deliver(&s, &p, &reply)
            .await
            .map_err(|e| format!("a reply to the phone: {e}"))?;
        never_delivered(
            &a,
            &p,
            &udp_packet(alpha, phone, b"not allowed to the phone")?,
        )
        .await
        .map_err(|e| format!("the server did not guard the phone: {e}"))?;
        never_delivered(
            &p,
            &a,
            &udp_packet(phone, alpha, b"not allowed from the phone")?,
        )
        .await
        .map_err(|e| format!("alpha let the phone in: {e}"))?;

        set_policy(&s, network, phone_policy(alpha_node)?).await?;
        never_delivered(&p, &s, &udp_packet(phone, server, b"server is off limits")?)
            .await
            .map_err(|e| format!("the server let the phone in against policy: {e}"))?;
        let resolver = Ipv4Addr::new(100, 64, 0, 1);
        deliver(&p, &s, &udp_ports(phone, 4000, resolver, 53, b"dns query")?)
            .await
            .map_err(|e| {
                format!("the phone's DNS was blocked by a policy that says nothing of it: {e}")
            })?;

        p.shutdown().await?;
        wait_for_peer(&a, network, "phone", Duration::from_secs(80), |p| !p.online)
            .await
            .map_err(|e| format!("alpha never saw the phone go offline: {e}"))?;

        let id = peer_info(&s, network, "phone").await?.node;
        if remove(&s, network, id).await != Response::DeviceRemoved {
            return Err("the phone could not be removed".into());
        }
        let p = phone_from(&everything).await?;
        never_delivered(&p, &s, &udp_packet(phone, server, b"after removal")?)
            .await
            .map_err(|e| format!("a removed phone still got through: {e}"))?;
        shutdown([p, a, s]).await
    }

    /// A `len`-byte IPv4 UDP packet, with don't-fragment set or not.
    fn big_udp(
        source: Ipv4Addr,
        destination: Ipv4Addr,
        len: usize,
        dont_fragment: bool,
    ) -> TestResult<Vec<u8>> {
        let mut packet = udp_packet(source, destination, &vec![9; len - 28])?;
        packet[6] = if dont_fragment { 0x40 } else { 0 };
        packet[10..12].fill(0);
        let mut sum: u32 = packet[..20]
            .chunks(2)
            .map(|word| u32::from(u16::from_be_bytes([word[0], word[1]])))
            .sum();
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        packet[10..12].copy_from_slice(&(!u16::try_from(sum)?).to_be_bytes());
        Ok(packet)
    }

    /// The MTU an ICMP "fragmentation needed" in `packet` reports, if it is one.
    fn reported_mtu(packet: &[u8]) -> Option<u16> {
        (packet.first()? >> 4 == 4
            && *packet.get(9)? == 1
            && *packet.get(20)? == 3
            && *packet.get(21)? == 4)
            .then(|| Some(u16::from_be_bytes([*packet.get(26)?, *packet.get(27)?])))
            .flatten()
    }

    /// Waits up to `within` for `node`'s TUN to emit a packet `pick` accepts, sending
    /// `nudge` from `from` every half second meanwhile so data keeps flowing.
    async fn wait_packet<T>(
        node: &NodeHandle,
        (from, nudge): (&NodeHandle, &[u8]),
        within: Duration,
        pick: impl Fn(&[u8]) -> Option<T>,
    ) -> TestResult<T> {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            from.send_packet(nudge.to_vec()).await?;
            if let Ok(packet) = timeout(Duration::from_millis(500), node.recv_packet()).await
                && let Some(found) = pick(&packet?)
            {
                return Ok(found);
            }
        }
        Err("the expected packet never came".into())
    }

    /// How long finding a path's size may take: a lost size costs two 5s probes, and the
    /// search takes a few steps after that.
    const SIZE_SEARCH_BUDGET: Duration = Duration::from_secs(90);

    #[tokio::test(flavor = "current_thread")]
    async fn a_path_too_small_for_the_tunnel_is_measured_and_senders_are_told() -> TestResult {
        let a = start().await?;
        let b = start().await?;
        a.limit_udp_size(Some(1248)).await?;
        b.limit_udp_size(Some(1248)).await?;
        let network = create(&a, "alpha").await?;
        join(&a, &b, network, "beta").await?;
        let alpha = peer(&b, network, "alpha").await?;
        let beta = peer(&a, network, "beta").await?;
        deliver(&a, &b, &udp_packet(alpha, beta, b"before")?).await?;

        let nudge = udp_packet(alpha, beta, b"keep the session busy")?;
        let refused = big_udp(alpha, beta, 1250, true)?;
        let mtu = wait_packet(&a, (&a, &refused), SIZE_SEARCH_BUDGET, |packet| {
            reported_mtu(packet)
        })
        .await?;
        if !(1208..=1216).contains(&mtu) {
            return Err(format!("the sender was told {mtu}, but the path carries 1216").into());
        }
        let info = peer_info(&a, network, "beta").await?;
        if info.path_mtu != Some(mtu) {
            return Err(format!("peers does not show the path size: {info:?}").into());
        }
        let split = big_udp(alpha, beta, 1250, false)?;
        let pieces = wait_packet(&b, (&a, &split), Duration::from_secs(10), |packet| {
            (packet.len() <= usize::from(mtu) && packet[6] & 0x20 != 0).then_some(())
        })
        .await;
        pieces.map_err(|e| format!("a fragmentable packet did not arrive split: {e}"))?;
        drop(nudge);
        shutdown([a, b]).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_phone_behind_a_small_path_is_measured_by_its_gateway() -> TestResult {
        let s = start_server(true, false, Some("localhost")).await?;
        let a = start().await?;
        let network = create(&s, "server").await?;
        join(&s, &a, network, "alpha").await?;
        let server: Ipv4Addr = peer(&a, network, "server").await?;
        let (phone, everything) = checked_configs(export(&s, network, "Phone", &[]).await?)?;
        let p = phone_from(&everything).await?;
        p.limit_udp_size(Some(1248)).await?;

        let deadline = tokio::time::Instant::now() + SIZE_SEARCH_BUDGET;
        let measured = loop {
            if tokio::time::Instant::now() >= deadline {
                return Err("the gateway never measured the phone's path".into());
            }
            p.send_packet(udp_packet(phone, server, b"keepalive")?)
                .await?;
            if let Ok(packet) = timeout(Duration::from_millis(300), p.recv_packet()).await {
                let mut packet = packet?;
                if packet.get(9) == Some(&1) && packet.get(20) == Some(&8) {
                    let (source, destination) = (packet[12..16].to_vec(), packet[16..20].to_vec());
                    packet[12..16].copy_from_slice(&destination);
                    packet[16..20].copy_from_slice(&source);
                    packet[20] = 0;
                    let sum = u32::from(u16::from_be_bytes([packet[22], packet[23]])) + 0x0800;
                    let sum = (sum & 0xffff) + (sum >> 16);
                    packet[22..24].copy_from_slice(&u16::try_from(sum)?.to_be_bytes());
                    p.send_packet(packet).await?;
                }
            }
            if let Some(mtu) = peer_info(&s, network, "phone").await?.path_mtu {
                break mtu;
            }
        };
        if !(1208..=1216).contains(&measured) {
            return Err(
                format!("the gateway measured {measured}, but the path carries 1216").into(),
            );
        }
        let refused = big_udp(server, phone, 1250, true)?;
        let told = wait_packet(&s, (&s, &refused), Duration::from_secs(10), reported_mtu).await?;
        if told != measured {
            return Err(format!("the sender was told {told}, not {measured}").into());
        }
        shutdown([p, a, s]).await
    }
}
