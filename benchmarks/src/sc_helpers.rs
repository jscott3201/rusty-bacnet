//! SC benchmark helpers: cert generation and SC client/server setup.

use std::sync::Arc;

use rcgen::{date_time_ymd, CertificateParams, Issuer, KeyPair};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_frame::Vmac;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::error::Error;

/// Generated certificate material for testing.
pub struct CertMaterial {
    pub ca_cert_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
}

#[derive(Clone, Copy)]
enum CertValidity {
    Valid,
    Expired,
    NotYetValid,
}

/// Generate self-signed CA, server, and client certificates.
///
/// Also installs a rustls crypto provider for generated-certificate tests.
pub fn generate_test_certs() -> CertMaterial {
    generate_test_certs_profile(
        vec!["localhost".into(), "127.0.0.1".into()],
        CertValidity::Valid,
        CertValidity::Valid,
    )
}

/// Generate certs whose server certificate is expired.
pub fn generate_test_certs_with_expired_server() -> CertMaterial {
    generate_test_certs_profile(
        vec!["localhost".into(), "127.0.0.1".into()],
        CertValidity::Expired,
        CertValidity::Valid,
    )
}

/// Generate certs whose server certificate is not valid yet.
pub fn generate_test_certs_with_not_yet_valid_server() -> CertMaterial {
    generate_test_certs_profile(
        vec!["localhost".into(), "127.0.0.1".into()],
        CertValidity::NotYetValid,
        CertValidity::Valid,
    )
}

/// Generate certs whose client certificate is expired.
pub fn generate_test_certs_with_expired_client() -> CertMaterial {
    generate_test_certs_profile(
        vec!["localhost".into(), "127.0.0.1".into()],
        CertValidity::Valid,
        CertValidity::Expired,
    )
}

/// Generate certs whose client certificate is not valid yet.
pub fn generate_test_certs_with_not_yet_valid_client() -> CertMaterial {
    generate_test_certs_profile(
        vec!["localhost".into(), "127.0.0.1".into()],
        CertValidity::Valid,
        CertValidity::NotYetValid,
    )
}

/// Generate certs whose server SAN does not match localhost.
pub fn generate_test_certs_with_wrong_server_name() -> CertMaterial {
    generate_test_certs_profile(
        vec!["wrong.local".into()],
        CertValidity::Valid,
        CertValidity::Valid,
    )
}

fn generate_test_certs_profile(
    server_sans: Vec<String>,
    server_validity: CertValidity,
    client_validity: CertValidity,
) -> CertMaterial {
    // Install the default crypto provider (ignore if already installed).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // CA — use empty SANs; CA certs don't need subject alt names.
    let mut ca_params =
        CertificateParams::new(Vec::<String>::new()).expect("empty SANs should not fail");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_issuer = Issuer::from_params(&ca_params, &ca_key);

    // Server
    let mut server_params = CertificateParams::new(server_sans).unwrap();
    apply_validity(&mut server_params, server_validity);
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_issuer).unwrap();

    // Client
    let mut client_params = CertificateParams::new(vec!["bacnet-client".into()]).unwrap();
    apply_validity(&mut client_params, client_validity);
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &ca_issuer).unwrap();

    CertMaterial {
        ca_cert_pem: ca_cert.pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_cert_pem: client_cert.pem(),
        client_key_pem: client_key.serialize_pem(),
    }
}

fn apply_validity(params: &mut CertificateParams, validity: CertValidity) {
    match validity {
        CertValidity::Valid => {}
        CertValidity::Expired => {
            params.not_before = date_time_ymd(2000, 1, 1);
            params.not_after = date_time_ymd(2000, 1, 2);
        }
        CertValidity::NotYetValid => {
            params.not_before = date_time_ymd(4090, 1, 1);
            params.not_after = date_time_ymd(4096, 1, 1);
        }
    }
}

/// Build a rustls ClientConfig that trusts the test CA.
pub fn make_client_tls_config(certs: &CertMaterial) -> Arc<rustls::ClientConfig> {
    try_make_client_tls_config(certs).unwrap()
}

/// Try to build a rustls ClientConfig that trusts the test CA.
pub fn try_make_client_tls_config(
    certs: &CertMaterial,
) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    if ca_certs.is_empty() {
        return Err("no CA certificates found".into());
    }
    for cert in ca_certs {
        root_store.add(cert).map_err(|e| e.to_string())?;
    }

    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(Arc::new(config))
}

/// Build a TLS 1.2-only ClientConfig for negative BACnet/SC tests.
pub fn make_client_tls12_config(certs: &CertMaterial) -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }

    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Arc::new(config)
}

/// Build a raw rustls ServerConfig that requires client certificates (mTLS).
///
/// Retained for independent TLS peers and compatibility. Already-mTLS hubs use
/// [`try_make_hub_tls_config`] instead; this raw helper's contract is unchanged.
pub fn make_server_tls_config_mtls(certs: &CertMaterial) -> Arc<rustls::ServerConfig> {
    try_make_server_tls_config_mtls(certs).unwrap()
}

/// Try to build a raw mTLS ServerConfig for independent TLS peers and compatibility.
pub fn try_make_server_tls_config_mtls(
    certs: &CertMaterial,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.server_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    if cert_chain.is_empty() {
        return Err("no server certificates found".into());
    }
    let key = PrivateKeyDer::from_pem_slice(certs.server_key_pem.as_bytes())
        .map_err(|e| e.to_string())?;

    // Build root cert store for client certificate verification.
    let mut client_auth_roots = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    if ca_certs.is_empty() {
        return Err("no CA certificates found".into());
    }
    for cert in ca_certs {
        client_auth_roots.add(cert).map_err(|e| e.to_string())?;
    }

    let client_verifier =
        rustls::server::WebPkiClientVerifier::builder(Arc::new(client_auth_roots))
            .build()
            .map_err(|e| e.to_string())?;

    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(cert_chain, key)
        .map_err(|e| e.to_string())?;

    Ok(Arc::new(config))
}

/// Parse in-memory PEM into owned DER and build validated SC hub TLS policy.
///
/// Returns configuration errors without binding or performing file/network I/O.
/// Unlike the raw peer helper, validates every certificate in the hub chain.
pub fn try_make_hub_tls_config(certs: &CertMaterial) -> Result<ScHubTlsConfig, Error> {
    let cert_chain = CertificateDer::pem_slice_iter(certs.server_cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse server certificates: {e}")))?;
    let key = PrivateKeyDer::from_pem_slice(certs.server_key_pem.as_bytes())
        .map_err(|e| Error::Encoding(format!("failed to parse server key: {e}")))?;
    let ca_certs = CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse CA certificates: {e}")))?;
    ScHubTlsConfig::from_der(ca_certs, cert_chain, key)
}

/// Build the strict local node policy from in-memory PEM, without file/network I/O.
pub fn try_make_node_tls_config(certs: &CertMaterial) -> Result<ScNodeTlsConfig, Error> {
    let chain = CertificateDer::pem_slice_iter(certs.client_cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse client certificates: {e}")))?;
    let key = PrivateKeyDer::from_pem_slice(certs.client_key_pem.as_bytes())
        .map_err(|e| Error::Encoding(format!("failed to parse client key: {e}")))?;
    let ca = CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse CA certificates: {e}")))?;
    ScNodeTlsConfig::from_der(ca, chain, key)
}

/// Build a raw rustls ClientConfig with client credentials for independent peers.
///
/// The client authenticates to the hub by including its certificate chain
/// and private key, satisfying the hub's client-auth requirement.
pub fn make_client_tls_config_mtls(certs: &CertMaterial) -> Arc<rustls::ClientConfig> {
    try_make_client_tls_config_mtls(certs).unwrap()
}

/// Try to build a rustls ClientConfig that presents a client certificate (mTLS).
pub fn try_make_client_tls_config_mtls(
    certs: &CertMaterial,
) -> Result<Arc<rustls::ClientConfig>, String> {
    try_make_client_tls_config_mtls_with_client_identity(certs, certs)
}

/// Build an mTLS ClientConfig that trusts one server CA and presents another identity.
pub fn make_client_tls_config_mtls_with_client_identity(
    trusted_server: &CertMaterial,
    client_identity: &CertMaterial,
) -> Arc<rustls::ClientConfig> {
    try_make_client_tls_config_mtls_with_client_identity(trusted_server, client_identity).unwrap()
}

/// Try to build an mTLS ClientConfig that trusts one server CA and presents another identity.
pub fn try_make_client_tls_config_mtls_with_client_identity(
    trusted_server: &CertMaterial,
    client_identity: &CertMaterial,
) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(trusted_server.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    if ca_certs.is_empty() {
        return Err("no CA certificates found".into());
    }
    for cert in ca_certs {
        root_store.add(cert).map_err(|e| e.to_string())?;
    }

    let client_cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(client_identity.client_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    if client_cert_chain.is_empty() {
        return Err("no client certificates found".into());
    }
    let client_key = PrivateKeyDer::from_pem_slice(client_identity.client_key_pem.as_bytes())
        .map_err(|e| e.to_string())?;

    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_cert_chain, client_key)
        .map_err(|e| e.to_string())?;

    Ok(Arc::new(config))
}

/// Start an SC hub with mTLS (client certificate required).
pub async fn start_sc_hub_mtls(certs: &CertMaterial, hub_vmac: Vmac) -> (ScHub, String) {
    let tls_config = try_make_hub_tls_config(certs).unwrap();
    let hub = ScHub::start_with_tls_config(
        "127.0.0.1:0",
        tls_config,
        hub_vmac,
        [0; 16],
        ScHubHandshakeTimeouts::default(),
    )
    .await
    .unwrap();
    let addr = hub.local_addr().unwrap();
    let url = format!("wss://localhost:{}", addr.port());
    (hub, url)
}

/// Create an SC transport connected to the hub with mTLS client cert.
pub async fn make_sc_transport_mtls(
    hub_url: &str,
    certs: &CertMaterial,
    vmac: Vmac,
) -> ScTransport<TlsWebSocket> {
    let tls_config = try_make_node_tls_config(certs).unwrap();
    let ws = TlsWebSocket::connect(hub_url, tls_config).await.unwrap();
    ScTransport::new(ws, vmac)
}
