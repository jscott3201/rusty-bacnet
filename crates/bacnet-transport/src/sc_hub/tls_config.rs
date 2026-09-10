//! Required, immutable TLS policy for native hubs.

use std::{fmt, sync::Arc};

use bacnet_types::error::Error;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

/// Validated hub credentials with explicit trust, mandatory client authentication,
/// and TLS 1.3-only local policy. Available with the `sc-tls` feature.
///
/// Construct from already loaded, owned DER; no filesystem or network I/O occurs.
/// Only the supplied CA certificates become rustls trust anchors. The constructor
/// checks certificate syntax and the hub's matching certificate/key, not local
/// certificate dates, issuer relationships, revocation, or BACnet identity policy.
/// Peers verify operational certificates during TLS. This is not certification
/// of the complete Annex AB security profile (which requires TLS 1.3 *support*).
///
/// Clones share the same constrained configuration. There is no raw configuration
/// getter, mutable access, or unchecked conversion from caller-managed TLS.
/// All public [`super::ScHub`] startup methods require this policy. Node/client
/// TLS configuration remains caller-managed and is not constrained by this type.
///
/// An executable, in-memory example (applications normally load site credentials):
///
/// ```
/// use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
/// use rcgen::{CertificateParams, Issuer, KeyPair};
/// use rustls::pki_types::PrivatePkcs8KeyDer;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
/// ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
/// let ca_key = KeyPair::generate()?;
/// let ca = ca_params.self_signed(&ca_key)?;
/// let issuer = Issuer::from_params(&ca_params, &ca_key);
/// let key = KeyPair::generate()?;
/// let cert = CertificateParams::new(vec!["localhost".into()])?.signed_by(&key, &issuer)?;
/// let tls = ScHubTlsConfig::from_der(
///     vec![ca.der().clone()], vec![cert.der().clone()],
///     PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
/// )?;
/// tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
///     let mut hub = ScHub::start_with_tls_config(
///         "127.0.0.1:0", tls, [0x12; 6], [0x34; 16], ScHubHandshakeTimeouts::default(),
///     ).await?;
///     assert!(hub.local_addr().is_some());
///     hub.stop().await;
///     Ok::<_, bacnet_types::error::Error>(())
/// })?;
/// # Ok(())
/// # }
/// ```
///
/// Raw acceptors cannot be converted into this type:
///
/// ```compile_fail,E0277
/// use bacnet_transport::sc_hub::ScHubTlsConfig;
/// fn convert(acceptor: tokio_rustls::TlsAcceptor) -> ScHubTlsConfig {
///     acceptor.into()
/// }
/// ```
///
/// Neither can an arbitrary rustls configuration:
///
/// ```compile_fail,E0277
/// use bacnet_transport::sc_hub::ScHubTlsConfig;
/// fn convert(config: rustls::ServerConfig) -> ScHubTlsConfig {
///     config.into()
/// }
/// ```
///
/// The inner policy cannot be constructed or extracted by a caller:
///
/// ```compile_fail,E0451
/// use bacnet_transport::sc_hub::ScHubTlsConfig;
/// fn bypass(inner: std::sync::Arc<rustls::ServerConfig>) -> ScHubTlsConfig {
///     ScHubTlsConfig { inner }
/// }
/// ```
///
/// ```compile_fail,E0616
/// use bacnet_transport::sc_hub::ScHubTlsConfig;
/// fn extract(config: ScHubTlsConfig) -> std::sync::Arc<rustls::ServerConfig> {
///     config.inner
/// }
/// ```
#[derive(Clone)]
pub struct ScHubTlsConfig {
    inner: Arc<rustls::ServerConfig>,
}

impl ScHubTlsConfig {
    /// Build the constrained policy before a hub can bind.
    ///
    /// `ca_certs` must be nonempty and every entry must be a usable rustls trust
    /// anchor. `cert_chain` must be nonempty, leaf first, with well-formed DER and
    /// a matching usable `key`. Errors retain configuration-stage context in
    /// [`Error::Encoding`]. No ambient/system trust is loaded.
    pub fn from_der(
        ca_certs: Vec<CertificateDer<'static>>,
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, Error> {
        if cert_chain.is_empty() {
            return Err(Error::Encoding("no server certificates found".into()));
        }
        if ca_certs.is_empty() {
            return Err(Error::Encoding("no CA certificates found".into()));
        }
        let mut roots = rustls::RootCertStore::empty();
        for cert in ca_certs {
            roots
                .add(cert)
                .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
        }
        for cert in &cert_chain {
            rustls::server::ParsedCertificate::try_from(cert)
                .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?;
        }
        // Fix the existing aws-lc provider rather than accepting a process-wide
        // custom key provider that might not establish certificate/key matching.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            provider.clone(),
        )
        .build()
        .map_err(|e| Error::Encoding(format!("failed to build client verifier: {e}")))?;
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?
            .with_client_cert_verifier(verifier)
            .with_single_cert(cert_chain, key)
            .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?;
        Ok(Self {
            inner: Arc::new(config),
        })
    }

    pub(super) fn into_acceptor(self) -> TlsAcceptor {
        TlsAcceptor::from(self.inner)
    }
}

impl fmt::Debug for ScHubTlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScHubTlsConfig").finish_non_exhaustive()
    }
}
