//! TLS and QUIC configuration.
//!
//! Both sides present their device certificate (mutual TLS 1.3). The certificate verifiers only check
//! that the peer owns the key of the certificate it shows; whether that certificate is trusted is
//! decided right after the handshake by comparing its fingerprint with the paired peers. That keeps
//! one endpoint for pairing and normal connections and lets us tell the other side *why* it was
//! refused ("not paired") instead of failing with an anonymous TLS alert.

use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

use crate::identity::{Fingerprint, Identity};

/// ALPN for normal connections between paired peers.
pub(crate) const ALPN_LINK: &[u8] = b"tablecross";
/// ALPN for pairing with a code.
pub(crate) const ALPN_PAIR: &[u8] = b"tablecross-pair";

/// QUIC gives up on its own after this long without any packet. Our heartbeat notices a lost peer
/// much earlier; this is only the safety net.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE: Duration = Duration::from_secs(1);

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts any certificate whose owner proves possession of the key.
#[derive(Debug)]
struct ProofOfKey {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ProofOfKey {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            algorithms: provider().signature_verification_algorithms,
        })
    }
}

impl ServerCertVerifier for ProofOfKey {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for ProofOfKey {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        ServerCertVerifier::verify_tls12_signature(self, message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        ServerCertVerifier::verify_tls13_signature(self, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ServerCertVerifier::supported_verify_schemes(self)
    }
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    transport
        .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("small duration")))
        .keep_alive_interval(Some(KEEP_ALIVE))
        // Only the control stream and a few transfers run at the same time
        .max_concurrent_bidi_streams(16u32.into())
        .max_concurrent_uni_streams(16u32.into());
    Arc::new(transport)
}

pub(crate) fn server_config(identity: &Identity) -> quinn::ServerConfig {
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .with_client_cert_verifier(ProofOfKey::new())
        .with_single_cert(vec![identity.cert()], identity.private_key())
        .expect("identity was validated when it was loaded");
    tls.alpn_protocols = vec![ALPN_LINK.to_vec(), ALPN_PAIR.to_vec()];
    let crypto = QuicServerConfig::try_from(Arc::new(tls)).expect("TLS 1.3 with an initial cipher suite");
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(transport());
    // Laptops change networks: let a known connection continue from a new address
    config.migration(true);
    config
}

pub(crate) fn client_config(identity: &Identity, alpn: &[u8]) -> quinn::ClientConfig {
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(ProofOfKey::new())
        .with_client_auth_cert(vec![identity.cert()], identity.private_key())
        .expect("identity was validated when it was loaded");
    tls.alpn_protocols = vec![alpn.to_vec()];
    let crypto = QuicClientConfig::try_from(Arc::new(tls)).expect("TLS 1.3 with an initial cipher suite");
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport());
    config
}

/// Fingerprint of the certificate the other side presented in the handshake.
pub(crate) fn peer_fingerprint(connection: &quinn::Connection) -> Option<Fingerprint> {
    let identity = connection.peer_identity()?;
    let certs = identity.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| Fingerprint::of(c))
}

/// ALPN the client asked for.
pub(crate) fn alpn(connection: &quinn::Connection) -> Option<Vec<u8>> {
    let data = connection.handshake_data()?;
    let data = data.downcast::<quinn::crypto::rustls::HandshakeData>().ok()?;
    data.protocol
}
