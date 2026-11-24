//! Immutable local TLS policy for the built-in SC node driver.

use std::{fmt, path::Path, sync::Arc};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::ScTlsConfigError;
use super::material::{Material, refused, verifier_refused};

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
/// Build it from the site's PEM files ([`Self::from_pem_files`]), from PEM
/// already in memory ([`Self::from_pem`]), or from owned DER
/// ([`Self::from_der`]). Only `from_pem_files` touches the filesystem; none
/// uses the network:
///
/// ```
/// use bacnet_transport::sc_tls::{ScNodeTlsConfig, ScTlsConfigError};
/// use rustls::pki_types::{CertificateDer, PrivateKeyDer};
/// fn policy(ca: Vec<CertificateDer<'static>>, chain: Vec<CertificateDer<'static>>,
///           key: PrivateKeyDer<'static>) -> Result<ScNodeTlsConfig, ScTlsConfigError> {
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
    /// Build the policy from owned DER: the site CA certificates to trust,
    /// this node's leaf-first chain, and the leaf's private key. Both lists
    /// must be nonempty and every certificate usable, and the key must load
    /// and belong to the leaf; [`ScTlsConfigError`] names the input that
    /// fails and why. No ambient trust, filesystem access, or networking is
    /// used. [`Self::from_pem`] and [`Self::from_pem_files`] take PEM.
    ///
    /// An owned `key` is cleared when this returns, on every path (a
    /// `PrivateKeyDer` that borrows static bytes cannot be). See
    /// [`Self::from_pem_files`], "Key memory".
    pub fn from_der(
        ca_certs: Vec<CertificateDer<'static>>,
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, ScTlsConfigError> {
        Self::build(Material::from_der(ca_certs, cert_chain, key)?)
    }

    /// Build the policy from PEM text: a CA bundle, this node's chain (leaf
    /// first), and exactly one unencrypted PKCS#8, SEC1 or PKCS#1 private
    /// key, in PEM that is well formed throughout. Then checked as
    /// [`Self::from_der`] checks.
    ///
    /// Text outside the PEM sections is ignored and CRLF line ends are
    /// taken. The CA bundle and chain take `CERTIFICATE` sections and skip
    /// every other kind: a key beside them is skipped, and so is an
    /// `X509 CRL`, so a bundle's revocation lists are not applied (no
    /// revocation checking is done). An input whose only certificates are
    /// OpenSSL `TRUSTED CERTIFICATE` or PKCS#7 sections is refused as
    /// [`ScTlsConfigError::UnsupportedCertificate`]. The key input skips
    /// certificates and EC parameters, so a key may share a file with them.
    ///
    /// The decoded key is cleared on return, but `private_key` is your
    /// buffer: clear it yourself. See [`Self::from_pem_files`], "Key memory".
    pub fn from_pem(
        ca_bundle: &[u8],
        cert_chain: &[u8],
        private_key: &[u8],
    ) -> Result<Self, ScTlsConfigError> {
        Self::build(Material::from_pem(ca_bundle, cert_chain, private_key)?)
    }

    /// Read three PEM files and build the policy as [`Self::from_pem`]
    /// does. The CA bundle is read and checked before the identity files
    /// are read. Each file is read once, here, up to
    /// [`MAX_PEM_FILE_BYTES`](super::MAX_PEM_FILE_BYTES); a later change to
    /// them takes a new configuration.
    ///
    /// # Key memory
    ///
    /// The key file's text and every key decoded from it are cleared when
    /// this returns, error paths included; rustls's key loader clears each
    /// copy it is handed. The configuration keeps no copy of the key's PEM
    /// or DER: it holds the parsed private key inside the TLS library,
    /// whose memory aws-lc clears when the configuration drops. Not
    /// cleared: the PEM decoder's transient buffers, which hold the key's
    /// base64 text while it decodes. A key given as PEM bytes or DER is
    /// also held by the caller, who must clear their own copy (`zeroize`);
    /// this call clears an owned DER key it takes.
    ///
    /// ```no_run
    /// use bacnet_transport::sc_tls::{ScNodeTlsConfig, ScTlsConfigError};
    /// fn policy() -> Result<ScNodeTlsConfig, ScTlsConfigError> {
    ///     ScNodeTlsConfig::from_pem_files("site-ca.pem", "node.pem", "node.key")
    /// }
    /// ```
    pub fn from_pem_files(
        ca_bundle: impl AsRef<Path>,
        cert_chain: impl AsRef<Path>,
        private_key: impl AsRef<Path>,
    ) -> Result<Self, ScTlsConfigError> {
        Self::build(Material::from_pem_files(
            ca_bundle.as_ref(),
            cert_chain.as_ref(),
            private_key.as_ref(),
        )?)
    }

    fn build(material: Material) -> Result<Self, ScTlsConfigError> {
        let Material {
            roots,
            chain,
            key,
            provider,
        } = material;
        // Same WebPKI policy as with_root_certificates, including its no-CRL
        // defaults. The stable wrapper observes evidence without deciding trust.
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::clone(&roots),
            Arc::clone(&provider),
        )
        .allow_unknown_revocation_status()
        .build()
        .map_err(verifier_refused)?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(refused)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(super::attempt_capture::ObservingVerifier(
                verifier,
            )))
            .with_client_auth_cert(chain.clone(), key.clone_key())
            .map_err(refused)?;
        // Server role for the opt-in direct listener: the same explicit CA
        // trust becomes the mandatory client verifier, the same node chain
        // and key become the server identity, and TLS 1.3-only stays fixed.
        // This enforces the same operational-certificate validation as the
        // dial path (explicit trust, peer verification during TLS) with the
        // roles reversed. Peers without a trusted operational certificate
        // fail the TLS handshake before any BACnet exchange.
        let verifier =
            rustls::server::WebPkiClientVerifier::builder_with_provider(roots, provider.clone())
                .build()
                .map_err(verifier_refused)?;
        let server_config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(refused)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(chain, key.clone_key())
            .map_err(refused)?;
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
