//! Immutable local TLS policy for the built-in SC node driver.

use std::{fmt, sync::Arc};

use bacnet_types::error::Error;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// Validated node credentials, explicit server trust, and TLS 1.3-only policy.
/// Available with `sc-tls`; required by the built-in [`super::TlsWebSocket`].
///
/// Only supplied CA certificates are trusted, with normal WebPKI server
/// certificate and name verification. The factory checks every certificate's
/// syntax and the matching operational key before any I/O, not local certificate
/// dates, issuer relationships, EKU, revocation, or BACnet identity authorization.
/// Peers perform certificate validation during TLS.
///
/// This is a **local configuration contract**, not proof that a remote hub
/// requests or verifies the node's identity. Credentials are supplied when
/// requested and compatible; a trusted server without a CertificateRequest can
/// complete the handshake. Normal TLS resumption is preserved and may not
/// retransmit certificates. Clones share the same configuration, verifier,
/// credential resolver, and resumption cache, including across reconnects.
///
/// This does not certify the full Annex AB profile. TLS 1.3-only is local policy;
/// the base Standard requires TLS 1.3 support. Other implementations of
/// [`crate::sc::WebSocketPort`] remain outside this built-in driver guarantee.
///
/// Load owned DER at the application boundary (no file/network I/O here):
///
/// ```
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// use rustls::pki_types::{CertificateDer, PrivateKeyDer};
/// fn policy(ca: Vec<CertificateDer<'static>>, chain: Vec<CertificateDer<'static>>,
///           key: PrivateKeyDer<'static>) -> Result<ScNodeTlsConfig, bacnet_types::error::Error> {
///     ScNodeTlsConfig::from_der(ca, chain, key)
/// }
/// ```
///
/// Arbitrary configurations and connectors cannot be converted:
///
/// ```compile_fail,E0277
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// fn bypass(config: rustls::ClientConfig) -> ScNodeTlsConfig { config.into() }
/// ```
/// ```compile_fail,E0277
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// fn bypass(config: std::sync::Arc<rustls::ClientConfig>) -> ScNodeTlsConfig { config.into() }
/// ```
/// ```compile_fail,E0277
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// fn bypass(connector: tokio_rustls::TlsConnector) -> ScNodeTlsConfig { connector.into() }
/// ```
/// ```compile_fail,E0451
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// fn bypass(inner: std::sync::Arc<rustls::ClientConfig>) -> ScNodeTlsConfig {
///     ScNodeTlsConfig { inner }
/// }
/// ```
/// ```compile_fail,E0616
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
/// fn extract(config: ScNodeTlsConfig) -> std::sync::Arc<rustls::ClientConfig> { config.inner }
/// ```
#[derive(Clone)]
pub struct ScNodeTlsConfig {
    inner: Arc<rustls::ClientConfig>,
    server_inner: Arc<rustls::ServerConfig>,
}

impl ScNodeTlsConfig {
    /// Validate owned, nonempty CA and leaf-first certificate chains and a
    /// usable matching private key. Configuration errors use [`Error::Encoding`].
    /// No ambient trust, filesystem access, or networking is used.
    pub fn from_der(
        ca_certs: Vec<CertificateDer<'static>>,
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, Error> {
        if cert_chain.is_empty() {
            return Err(Error::Encoding("no client certificates found".into()));
        }
        if ca_certs.is_empty() {
            return Err(Error::Encoding("no CA certificates found".into()));
        }
        let mut roots = rustls::RootCertStore::empty();
        for cert in ca_certs.clone() {
            roots
                .add(cert)
                .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
        }
        for cert in &cert_chain {
            rustls::server::ParsedCertificate::try_from(cert)
                .map_err(|e| Error::Encoding(format!("TLS client auth error: {e}")))?;
        }
        // Match the hub's fixed provider: caller/global key providers cannot
        // weaken the built-in provider's certificate/key consistency check.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| Error::Encoding(format!("TLS client auth error: {e}")))?
            .with_root_certificates(roots)
            .with_client_auth_cert(cert_chain.clone(), key.clone_key())
            .map_err(|e| Error::Encoding(format!("TLS client auth error: {e}")))?;
        // Server role for the opt-in direct listener: the same explicit CA
        // trust becomes the mandatory client verifier, the same node chain
        // and key become the server identity, and TLS 1.3-only stays fixed.
        // This enforces the same operational-certificate validation as the
        // dial path (explicit trust, peer verification during TLS) with the
        // roles reversed. Peers without a trusted operational certificate
        // fail the TLS handshake before any BACnet exchange.
        let mut server_roots = rustls::RootCertStore::empty();
        for cert in ca_certs {
            server_roots
                .add(cert)
                .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(server_roots),
            provider.clone(),
        )
        .build()
        .map_err(|e| Error::Encoding(format!("TLS server auth error: {e}")))?;
        let server_config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| Error::Encoding(format!("TLS server auth error: {e}")))?
            .with_client_cert_verifier(verifier)
            .with_single_cert(cert_chain, key)
            .map_err(|e| Error::Encoding(format!("TLS server auth error: {e}")))?;
        Ok(Self {
            inner: Arc::new(config),
            server_inner: Arc::new(server_config),
        })
    }

    pub(super) fn into_connector(self) -> TlsConnector {
        TlsConnector::from(self.inner)
    }

    /// Acceptor for the opt-in direct listener (server role, same policy).
    ///
    /// Clones share the constrained server configuration with mandatory
    /// operational-client verification; no raw acceptor is exposed.
    pub(super) fn acceptor(&self) -> TlsAcceptor {
        TlsAcceptor::from(Arc::clone(&self.server_inner))
    }
}

impl fmt::Debug for ScNodeTlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScNodeTlsConfig").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "tls_config_tests.rs"]
mod tests;
