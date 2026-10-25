//! End-to-end over real TLS on loopback with a hand-built self-signed Ed25519 certificate.

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use kith_transport_ws::{ClientOptions, Error, TlsIdentity, WsServer, connect};
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    const SEED: [u8; 32] = [42; 32];

    /// X.509 v3, CN=kith, valid 2025..2049. The signature is zeros: a pinned client compares
    /// bytes and never checks it, and rustls only reads the key from it.
    fn certificate(public_key: [u8; 32]) -> Vec<u8> {
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
        cert
    }

    fn private_key() -> Vec<u8> {
        let mut key = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        key.extend_from_slice(&SEED);
        key
    }

    fn datagram(i: usize) -> Vec<u8> {
        (0..(i * 263) % 9000)
            .map(|j| u8::try_from((i ^ j) % 256).unwrap())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn datagrams_cross_tls_both_ways_and_wrong_paths_get_404() {
        let public_key = kith_crypto::SigningKey::from_seed(&SEED)
            .verifying_key()
            .to_bytes();
        let cert = CertificateDer::from(certificate(public_key));
        let identity = TlsIdentity {
            chain: vec![cert.clone()],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key())),
        };
        let server = WsServer::bind(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            "/s3cret/ws",
            Some(identity),
        )
        .await
        .unwrap();
        let port = server.local_addr().unwrap().port();

        tokio::spawn(async move {
            loop {
                let incoming = server.accept().await.unwrap();
                tokio::spawn(async move {
                    if let Ok(Some(mut link)) = incoming.upgrade().await {
                        while let Ok(datagram) = link.recv().await {
                            link.send(&datagram).await.unwrap();
                        }
                    }
                });
            }
        });

        let options = ClientOptions {
            server_name: Some("localhost".to_owned()),
            pinned_certificate: Some(cert),
            ..ClientOptions::default()
        };

        let wrong = connect(&format!("wss://127.0.0.1:{port}/"), &options).await;
        assert!(matches!(wrong, Err(Error::Rejected(404))), "{wrong:?}");

        let mut link = connect(&format!("wss://127.0.0.1:{port}/s3cret/ws"), &options)
            .await
            .unwrap();
        let sender = link.sender();
        let sending = tokio::spawn(async move {
            for i in 0..500 {
                sender.send(&datagram(i)).await.unwrap();
            }
        });
        for i in 0..500 {
            assert_eq!(link.recv().await.unwrap(), datagram(i));
        }
        sending.await.unwrap();
    }
}
