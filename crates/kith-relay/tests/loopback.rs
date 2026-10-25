//! End-to-end relay scenarios over loopback TCP and TLS.

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use kith_crypto::SigningKey;
    use kith_relay::{
        Client, ClientOptions, Incoming, RejectReason, Server, ServerOptions, Status, TlsIdentity,
        Verification,
    };
    use kith_types::NodeId;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::time::timeout;

    const TLS_SEED: [u8; 32] = [42; 32];
    const WAIT: Duration = Duration::from_secs(5);

    /// X.509 v3, CN=kith, valid 2025..2049. The signature is zeros: a pinned client compares
    /// bytes and never checks it, and rustls only reads the key from it.
    fn certificate(public_key: [u8; 32]) -> CertificateDer<'static> {
        let name = [
            0x30, 0x0f, 0x31, 0x0d, 0x30, 0x0b, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x04, b'k',
            b'i', b't', b'h',
        ];
        let ed25519 = [0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70];
        let mut tbs = vec![0x30, 0x7d, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x01, 0x01];
        tbs.extend_from_slice(&ed25519);
        tbs.extend_from_slice(&name);
        tbs.extend_from_slice(&[0x30, 0x1e, 0x17, 0x0d]);
        tbs.extend_from_slice(b"250101000000Z");
        tbs.extend_from_slice(&[0x17, 0x0d]);
        tbs.extend_from_slice(b"491231235959Z");
        tbs.extend_from_slice(&name);
        tbs.extend_from_slice(&[0x30, 0x2a]);
        tbs.extend_from_slice(&ed25519);
        tbs.extend_from_slice(&[0x03, 0x21, 0x00]);
        tbs.extend_from_slice(&public_key);
        let mut cert = vec![0x30, 0x81, 0xc9];
        cert.extend_from_slice(&tbs);
        cert.extend_from_slice(&ed25519);
        cert.extend_from_slice(&[0x03, 0x41, 0x00]);
        cert.extend_from_slice(&[0; 64]);
        CertificateDer::from(cert)
    }

    fn tls_identity() -> TlsIdentity {
        let public_key = SigningKey::from_seed(&TLS_SEED).verifying_key().to_bytes();
        let mut key = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        key.extend_from_slice(&TLS_SEED);
        TlsIdentity {
            chain: vec![certificate(public_key)],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
        }
    }

    fn key(seed: u8) -> Arc<SigningKey> {
        Arc::new(SigningKey::from_seed(&[seed; 32]))
    }

    fn fast() -> ClientOptions {
        ClientOptions {
            min_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(200),
            ..ClientOptions::default()
        }
    }

    async fn relay(addr: SocketAddr, members: Vec<NodeId>, tls: Option<TlsIdentity>) -> Server {
        let authorizer = move |node: &NodeId| members.contains(node);
        Server::bind(addr, relay_id(), authorizer, tls, ServerOptions::default())
            .await
            .unwrap()
    }

    fn relay_id() -> NodeId {
        key(1).node_id()
    }

    async fn until(client: &Client, wanted: Status) {
        let mut status = client.watch_status();
        timeout(WAIT, status.wait_for(|s| *s == wanted))
            .await
            .unwrap()
            .unwrap();
    }

    async fn recv(incoming: &mut Incoming) -> (NodeId, Vec<u8>) {
        timeout(WAIT, incoming.recv()).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn members_exchange_datagrams_over_tls_with_authenticated_senders() {
        let (a, b) = (key(2), key(3));
        let server = relay(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            vec![a.node_id(), b.node_id()],
            Some(tls_identity()),
        )
        .await;
        let address = server.local_addr().unwrap().to_string();
        let pinned = Verification::Pinned(tls_identity().chain.remove(0));
        let connect = |key: &Arc<SigningKey>| {
            let tls = |k| {
                Client::tls(
                    address.clone(),
                    "localhost",
                    pinned.clone(),
                    relay_id(),
                    k,
                    fast(),
                )
            };
            tls(Arc::clone(key)).unwrap()
        };
        let (client_a, mut incoming_a) = connect(&a);
        let (client_b, mut incoming_b) = connect(&b);
        until(&client_a, Status::Connected).await;
        until(&client_b, Status::Connected).await;

        // Sizes span a single read chunk, several chunks, and the protocol maximum.
        let sizes = [0, 1, 1400, 20_000, 65_503];
        for (i, &size) in sizes.iter().enumerate() {
            let datagram: Vec<u8> = (0..size)
                .map(|j| u8::try_from((i * 31 + j) % 251).unwrap())
                .collect();
            client_a.send(b.node_id(), datagram.clone()).unwrap();
            assert_eq!(recv(&mut incoming_b).await, (a.node_id(), datagram));
        }
        client_b.send(a.node_id(), b"reply".to_vec()).unwrap();
        assert_eq!(
            recv(&mut incoming_a).await,
            (b.node_id(), b"reply".to_vec())
        );
    }

    #[tokio::test]
    async fn non_member_is_rejected_and_stops_redialing() {
        let server = relay(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            vec![key(2).node_id()],
            None,
        )
        .await;
        let address = server.local_addr().unwrap().to_string();
        let (outsider, _incoming) = Client::tcp(address, relay_id(), key(9), fast());
        until(&outsider, Status::Rejected(RejectReason::NOT_MEMBER)).await;
        assert!(outsider.send(key(2).node_id(), vec![1]).is_err());
    }

    #[tokio::test]
    async fn client_reconnects_after_server_restart() {
        let (a, b) = (key(2), key(3));
        let members = vec![a.node_id(), b.node_id()];
        let server = relay(SocketAddr::from(([127, 0, 0, 1], 0)), members.clone(), None).await;
        let addr = server.local_addr().unwrap();
        let (client_a, _incoming_a) =
            Client::tcp(addr.to_string(), relay_id(), Arc::clone(&a), fast());
        let (client_b, mut incoming_b) =
            Client::tcp(addr.to_string(), relay_id(), Arc::clone(&b), fast());
        until(&client_a, Status::Connected).await;
        until(&client_b, Status::Connected).await;

        drop(server);
        until(&client_a, Status::Connecting).await;
        let _server = relay(addr, members, None).await;
        until(&client_a, Status::Connected).await;
        until(&client_b, Status::Connected).await;

        client_a
            .send(b.node_id(), b"after restart".to_vec())
            .unwrap();
        assert_eq!(
            recv(&mut incoming_b).await,
            (a.node_id(), b"after restart".to_vec())
        );
    }
}
