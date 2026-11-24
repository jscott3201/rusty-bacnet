//! Required, immutable TLS policy for native hubs.

use std::{fmt, path::Path, sync::Arc};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use crate::sc_tls::ScTlsConfigError;
use crate::sc_tls::material::{Material, refused, verifier_refused};

/// Validated hub credentials with explicit trust, mandatory client authentication,
/// and TLS 1.3-only local policy. Available with the `sc-tls` feature.
///
/// Construct from the site's PEM files ([`Self::from_pem_files`]), from PEM in
/// memory ([`Self::from_pem`]), or from owned DER ([`Self::from_der`]); only
/// `from_pem_files` touches the filesystem, and none the network.
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
/// the fixed identity below is TEST-ONLY. Provision and durably reuse the hosting
/// device UUID before deployment; all startup APIs reject zero UUID/reserved VMAC.
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
    certificate_bindings: Option<super::ScHubCertificateBindings>,
    broadcast_rate: super::ScHubBroadcastRatePolicy,
    admission_limits: super::ScHubAdmissionLimits,
    admission_policy: Option<super::ScHubAdmissionPolicy>,
    graceful_timeouts: super::ScHubGracefulTimeouts,
    probe_policy: super::ScHubProbePolicy,
    relay_send_budget: std::time::Duration,
}

impl ScHubTlsConfig {
    /// Build the constrained policy before a hub can bind, from owned DER:
    /// the site CA certificates to trust, the hub's leaf-first chain, and
    /// the leaf's private key. Both lists must be nonempty and every
    /// certificate usable, and the key must load and belong to the leaf;
    /// [`ScTlsConfigError`] names the input that fails and why. No
    /// ambient/system trust is loaded. [`Self::from_pem`] and
    /// [`Self::from_pem_files`] take PEM. An owned `key` is cleared when
    /// this returns, on every path (a `PrivateKeyDer` that borrows static
    /// bytes cannot be).
    pub fn from_der(
        ca_certs: Vec<CertificateDer<'static>>,
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, ScTlsConfigError> {
        Self::build(Material::from_der(ca_certs, cert_chain, key)?)
    }

    /// Build the policy from PEM text: a CA bundle, the hub's chain (leaf
    /// first), and exactly one unencrypted PKCS#8, SEC1 or PKCS#1 private
    /// key, decoded as [`ScNodeTlsConfig::from_pem`] decodes them, then
    /// checked as [`Self::from_der`] checks. Keys and `X509 CRL` sections in
    /// the CA bundle or chain are skipped, so no revocation list applies.
    /// The decoded key is cleared on return, but `private_key` is your
    /// buffer: clear it yourself.
    ///
    /// [`ScNodeTlsConfig::from_pem`]: crate::sc_tls::ScNodeTlsConfig::from_pem
    pub fn from_pem(
        ca_bundle: &[u8],
        cert_chain: &[u8],
        private_key: &[u8],
    ) -> Result<Self, ScTlsConfigError> {
        Self::build(Material::from_pem(ca_bundle, cert_chain, private_key)?)
    }

    /// Read three PEM files and build the policy as [`Self::from_pem`]
    /// does. The CA bundle is read and checked before the identity files
    /// are read; each file is read once, here, up to
    /// [`MAX_PEM_FILE_BYTES`](crate::sc_tls::MAX_PEM_FILE_BYTES).
    ///
    /// # Key memory
    ///
    /// As [`ScNodeTlsConfig::from_pem_files`]: the key file's text and
    /// every decoded key are cleared on return, and the configuration keeps
    /// no copy of the key's PEM or DER. The PEM decoder's transient
    /// buffers are not cleared.
    ///
    /// [`ScNodeTlsConfig::from_pem_files`]: crate::sc_tls::ScNodeTlsConfig::from_pem_files
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
        let verifier =
            rustls::server::WebPkiClientVerifier::builder_with_provider(roots, provider.clone())
                .build()
                .map_err(verifier_refused)?;
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(refused)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(chain, key.clone_key())
            .map_err(refused)?;
        Ok(Self {
            inner: Arc::new(config),
            certificate_bindings: None,
            broadcast_rate: super::ScHubBroadcastRatePolicy::default(),
            admission_limits: super::ScHubAdmissionLimits::default(),
            admission_policy: None,
            graceful_timeouts: super::ScHubGracefulTimeouts::default(),
            probe_policy: super::ScHubProbePolicy::default(),
            relay_send_budget: std::time::Duration::from_secs(5),
        })
    }

    /// Enable mapped-only certificate-to-claim admission, including offline
    /// reservations. An absent map retains CA-valid admission. Existing admin
    /// policy remains conjunctive and can further deny a matched leaf.
    pub fn with_certificate_bindings(mut self, bindings: super::ScHubCertificateBindings) -> Self {
        self.certificate_bindings = Some(bindings);
        self
    }

    /// Immutable configured installation policy, if enabled.
    pub fn certificate_bindings(&self) -> Option<&super::ScHubCertificateBindings> {
        self.certificate_bindings.as_ref()
    }

    /// Tune the always-on hub broadcast relay budgets without changing TLS policy.
    ///
    /// Every startup validates these bounds before binding. Zero or overflowing
    /// bounds are configuration errors, not a way to disable limiting. Each hub
    /// started from a clone gets independent buckets and drop counters. See
    /// [`super::ScHubBroadcastRatePolicy`] for defaults, units and tuning guidance.
    pub fn with_broadcast_rate_policy(mut self, policy: super::ScHubBroadcastRatePolicy) -> Self {
        self.broadcast_rate = policy;
        self
    }

    /// The broadcast policy that will be validated at startup.
    pub fn broadcast_rate_policy(&self) -> super::ScHubBroadcastRatePolicy {
        self.broadcast_rate
    }

    /// Tune hub admission bounds without changing TLS policy.
    ///
    /// Every startup validates these bounds before binding. Zero or
    /// overflowing bounds are configuration errors, not a way to disable
    /// limiting. Each hub started from a clone gets independent admission
    /// and deny counters. See [`super::ScHubAdmissionLimits`] for defaults
    /// (256 established clients + 256 handshakes, preserving the previous
    /// 512-connection total) and the split between the two caps.
    pub fn with_admission_limits(mut self, limits: super::ScHubAdmissionLimits) -> Self {
        self.admission_limits = limits;
        self
    }

    /// The admission bounds that will be validated at startup.
    pub fn admission_limits(&self) -> super::ScHubAdmissionLimits {
        self.admission_limits
    }

    /// Install an admin admission policy without changing TLS policy.
    ///
    /// The policy sees the bounded [`super::ScHubAdmissionInput`] (claimed
    /// VMAC/UUID/limits inside the TLS channel plus the verified-client
    /// boolean and RB-07 peer context, plus the current redacted registration
    /// kind) and returns
    /// [`super::ScHubAdmissionDecision::Allow`] or
    /// [`super::ScHubAdmissionDecision::Deny`]. It runs synchronously under
    /// the registry lock before the registration commit, so it must not
    /// block, perform I/O, or await; a panicking policy fails closed to
    /// Deny. A deny answers with the existing `RESOURCES`/`OTHER` NAK
    /// family, closes without map mutation, leaves any incumbent untouched,
    /// and bumps the hub's saturating deny counter. Absent policy means
    /// allow-all; configured limits still apply either way. Clones share
    /// the policy but never the deny counters. Device UUID shape and
    /// equality are registry keys, not certificate authentication: no
    /// certificate subject or fingerprint is exposed to the callback. Optional
    /// certificate bindings independently constrain these claims.
    ///
    /// Refusing replacement is an explicit local security policy before
    /// protocol acceptance; the default retains Annex AB known-UUID replacement.
    /// No incumbent identity fields are exposed by the classification.
    ///
    /// ```
    /// use bacnet_transport::sc_hub::{ScHubAdmissionDecision as Decision,
    ///     ScHubRegistrationKind as Kind, ScHubTlsConfig};
    /// fn preserve_incumbents(tls: ScHubTlsConfig) -> ScHubTlsConfig {
    ///     tls.with_admission_policy(|input| match input.registration {
    ///         Kind::SameUuidSameVmac | Kind::SameUuidMovedVmac => Decision::Deny,
    ///         _ => Decision::Allow,
    ///     })
    /// }
    /// ```
    pub fn with_admission_policy(
        mut self,
        policy: impl Fn(&super::ScHubAdmissionInput) -> super::ScHubAdmissionDecision
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.admission_policy = Some(Arc::new(policy));
        self
    }

    pub(super) fn admission_policy(&self) -> Option<super::ScHubAdmissionPolicy> {
        self.admission_policy.clone()
    }

    /// Tune graceful-shutdown bounds without changing TLS policy.
    ///
    /// Every startup validates these bounds before binding (per-peer
    /// Disconnect-Ack, per-peer AB.7.5.5 close, and overall drain; overall
    /// must cover ack + close). Each hub started from a clone uses the same
    /// bound values. See [`super::ScHubGracefulTimeouts`] for defaults
    /// (5s ack + 5s close within 15s overall) and
    /// [`super::ScHub::shutdown_gracefully`] for the exchange order.
    pub fn with_graceful_timeouts(mut self, timeouts: super::ScHubGracefulTimeouts) -> Self {
        self.graceful_timeouts = timeouts;
        self
    }

    /// The graceful bounds that will be validated at startup.
    pub fn graceful_timeouts(&self) -> super::ScHubGracefulTimeouts {
        self.graceful_timeouts
    }

    /// Configure optional Hub-originated probes, independently of node keepalive.
    ///
    /// ```
    /// use std::time::Duration;
    /// use bacnet_transport::sc_hub::{ScHubProbePolicy, ScHubTlsConfig};
    /// fn configure(tls: ScHubTlsConfig) -> Result<ScHubTlsConfig, bacnet_types::error::Error> {
    ///     let probe = ScHubProbePolicy::new(Duration::from_secs(2),
    ///         Duration::from_secs(10), Duration::from_secs(3), Duration::from_secs(1))?;
    ///     tls.with_probe_policy(probe).with_relay_send_budget(Duration::from_millis(750))
    /// }
    /// ```
    pub fn with_probe_policy(mut self, policy: super::ScHubProbePolicy) -> Self {
        self.probe_policy = policy;
        self
    }

    /// The configured scan-driven Hub probe policy.
    pub fn probe_policy(&self) -> super::ScHubProbePolicy {
        self.probe_policy
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

impl ScHubTlsConfig {
    /// Validate the transit relay budget before TLS file I/O or bind.
    /// Positive whole milliseconds must be at most `i64::MAX` (elapsed-tick
    /// headroom) and fit the platform monotonic clock.
    pub fn validate_relay_send_budget(
        budget: std::time::Duration,
    ) -> Result<(), bacnet_types::error::Error> {
        super::timing::validate_milliseconds("relay send budget", budget)
    }

    /// Set one acquisition-plus-send budget for every transit relay attempt.
    /// Defaults to five seconds. Timeout does not retire, retry or fabricate a
    /// Result; it cannot retract already buffered WebSocket bytes. Applies to
    /// NPDU/opaque unicast, each concurrent broadcast recipient, and forwarded
    /// BVLC-Result. Probe, control, cleanup and graceful-shutdown policies remain
    /// separate. Existing outcome counters remain limited to eligible unicast.
    pub fn with_relay_send_budget(
        mut self,
        budget: std::time::Duration,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::validate_relay_send_budget(budget)?;
        self.relay_send_budget = budget;
        Ok(self)
    }

    /// Configured transit relay acquisition-plus-send budget.
    pub fn relay_send_budget(&self) -> std::time::Duration {
        self.relay_send_budget
    }
}
