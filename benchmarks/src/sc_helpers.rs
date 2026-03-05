//! SC benchmark helpers: cert generation and SC client/server setup.

use std::sync::Arc;

use rcgen::{CertificateParams, Issuer, KeyPair};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_frame::Vmac;
use bacnet_transport::sc_hub::ScHub;
use bacnet_transport::sc_tls::TlsWebSocket;

/// Generated certificate material for testing.
pub struct CertMaterial {
    pub ca_cert_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
}

/// Generate self-signed CA, server, and client certificates.
///
/// Also installs the `ring` crypto provider for rustls (required when both
/// `ring` and `aws-lc-rs` features are enabled).
pub fn generate_test_certs() -> CertMaterial {
    // Install ring as the default crypto provider (ignore if already installed).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // CA — use empty SANs; CA certs don't need subject alt names.
    let mut ca_params =
        CertificateParams::new(Vec::<String>::new()).expect("empty SANs should not fail");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_issuer = Issuer::from_params(&ca_params, &ca_key);

    // Server
    let server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_issuer).unwrap();

    // Client
    let client_params = CertificateParams::new(vec!["bacnet-client".into()]).unwrap();
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

/// Build a rustls ServerConfig from cert material.
pub fn make_server_tls_config(certs: &CertMaterial) -> Arc<rustls::ServerConfig> {
    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.server_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let key = PrivateKeyDer::from_pem_slice(certs.server_key_pem.as_bytes()).unwrap();

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .unwrap();

    Arc::new(config)
}

/// Build a rustls ClientConfig that trusts the test CA.
pub fn make_client_tls_config(certs: &CertMaterial) -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Arc::new(config)
}

/// Build a rustls ServerConfig that requires client certificates (mTLS).
///
/// Per ASHRAE 135-2020 Annex AB.3, the hub verifies client certificates
/// against the trusted CA to enforce mutual TLS authentication.
pub fn make_server_tls_config_mtls(certs: &CertMaterial) -> Arc<rustls::ServerConfig> {
    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.server_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let key = PrivateKeyDer::from_pem_slice(certs.server_key_pem.as_bytes()).unwrap();

    // Build root cert store for client certificate verification.
    let mut client_auth_roots = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    for cert in ca_certs {
        client_auth_roots.add(cert).unwrap();
    }

    let client_verifier =
        rustls::server::WebPkiClientVerifier::builder(Arc::new(client_auth_roots))
            .build()
            .unwrap();

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(cert_chain, key)
        .unwrap();

    Arc::new(config)
}

/// Build a rustls ClientConfig that presents a client certificate (mTLS).
///
/// The client authenticates to the hub by including its certificate chain
/// and private key, satisfying the hub's client-auth requirement.
pub fn make_client_tls_config_mtls(certs: &CertMaterial) -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.ca_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }

    let client_cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certs.client_cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let client_key = PrivateKeyDer::from_pem_slice(certs.client_key_pem.as_bytes()).unwrap();

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_cert_chain, client_key)
        .unwrap();

    Arc::new(config)
}

/// Start an SC hub on an ephemeral port.
pub async fn start_sc_hub(certs: &CertMaterial, hub_vmac: Vmac) -> (ScHub, String) {
    let tls_config = make_server_tls_config(certs);
    let acceptor = TlsAcceptor::from(tls_config);
    let hub = ScHub::start("127.0.0.1:0", acceptor, hub_vmac)
        .await
        .unwrap();
    let addr = hub.local_addr().unwrap();
    let url = format!("wss://localhost:{}", addr.port());
    (hub, url)
}

/// Create an SC transport connected to the hub.
pub async fn make_sc_transport(
    hub_url: &str,
    certs: &CertMaterial,
    vmac: Vmac,
) -> ScTransport<TlsWebSocket> {
    let tls_config = make_client_tls_config(certs);
    let ws = TlsWebSocket::connect(hub_url, tls_config).await.unwrap();
    ScTransport::new(ws, vmac)
}

/// Start an SC hub with mTLS (client certificate required).
pub async fn start_sc_hub_mtls(certs: &CertMaterial, hub_vmac: Vmac) -> (ScHub, String) {
    let tls_config = make_server_tls_config_mtls(certs);
    let acceptor = TlsAcceptor::from(tls_config);
    let hub = ScHub::start("127.0.0.1:0", acceptor, hub_vmac)
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
    let tls_config = make_client_tls_config_mtls(certs);
    let ws = TlsWebSocket::connect(hub_url, tls_config).await.unwrap();
    ScTransport::new(ws, vmac)
}
