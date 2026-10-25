//! TLS 1.3 raw-public-key (RFC 7250) authentication keyed to kith identities.
//!
//! No CA, no certificate validation. Each side presents its Ed25519 key as a bare
//! `SubjectPublicKeyInfo`; the client pins the server to an expected [`NodeId`] and the server
//! accepts any valid key, leaving membership to the caller.

use std::sync::Arc;

use kith_crypto::VerifyingKey;
use kith_types::NodeId;
use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls13_signature_with_raw_key};
use rustls::pki_types::{
    CertificateDer, PrivatePkcs8KeyDer, ServerName, SubjectPublicKeyInfoDer, UnixTime,
};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::sign::CertifiedKey;
use rustls::{CertificateError, DistinguishedName, Error as TlsError, SignatureScheme};

use crate::{ALPN, QuicError, der};

/// Builds our own identity's raw-public-key credential, reused for every connection.
pub(crate) fn certified_key(
    seed: &[u8; 32],
    public_key: &[u8; 32],
) -> Result<Arc<CertifiedKey>, QuicError> {
    let pkcs8 = der::pkcs8(seed);
    let key_der = PrivatePkcs8KeyDer::from(pkcs8.as_ref());
    let signing_key = rustls::crypto::ring::sign::any_eddsa_type(&key_der)?;
    let spki = der::spki(public_key);
    let cert = CertificateDer::from(spki.to_vec());
    Ok(Arc::new(CertifiedKey::new(vec![cert], signing_key)))
}

/// Server-side rustls config: presents our key, accepts any valid client key.
pub(crate) fn server_config(
    provider: &Arc<CryptoProvider>,
    identity: Arc<CertifiedKey>,
) -> Result<rustls::ServerConfig, QuicError> {
    let verifier = Arc::new(AnyRawKeyClient {
        provider: Arc::clone(provider),
    });
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::clone(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(
            rustls::server::AlwaysResolvesServerRawPublicKeys::new(identity),
        ));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// Client-side rustls config: presents our key, pins the server to `expected`.
pub(crate) fn client_config(
    provider: &Arc<CryptoProvider>,
    identity: Arc<CertifiedKey>,
    expected: NodeId,
) -> Result<rustls::ClientConfig, QuicError> {
    let expected_spki = der::spki(&VerifyingKey::from_node_id(expected)?.to_bytes());
    let verifier = Arc::new(PinnedRawKeyServer {
        expected_spki,
        provider: Arc::clone(provider),
    });
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::clone(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(
            rustls::client::AlwaysResolvesClientRawPublicKeys::new(identity),
        ));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// Verifies the server presents exactly the expected peer's raw public key.
#[derive(Debug)]
struct PinnedRawKeyServer {
    expected_spki: [u8; der::SPKI_LEN],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedRawKeyServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if end_entity.as_ref() == self.expected_spki {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(CertificateError::ApplicationVerificationFailure.into())
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Accepts any well-formed Ed25519 raw public key; the caller decides membership.
#[derive(Debug)]
struct AnyRawKeyClient {
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for AnyRawKeyClient {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        der::node_id_from_spki(end_entity.as_ref())
            .map(|_| ClientCertVerified::assertion())
            .map_err(|_| CertificateError::BadEncoding.into())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}
