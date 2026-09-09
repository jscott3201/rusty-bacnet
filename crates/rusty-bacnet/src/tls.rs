//! TLS configuration helpers for Python bindings.

use bacnet_types::error::Error;
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Build a TLS 1.3 server config requiring client certificates (for Python ScHub).
pub fn build_server_tls_config(
    cert_path: &str,
    key_path: &str,
    ca_cert_path: &str,
) -> Result<Arc<tokio_rustls::rustls::ServerConfig>, Error> {
    use tokio_rustls::rustls;

    let cert_data = std::fs::read(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read server cert: {e}")))?;
    let key_data = std::fs::read(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read server key: {e}")))?;

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse server cert: {e}")))?;
    if certs.is_empty() {
        return Err(Error::Encoding("no server certificates found".into()));
    }
    let key = PrivateKeyDer::from_pem_slice(&key_data)
        .map_err(|e| Error::Encoding(format!("failed to parse server key: {e}")))?;

    let ca_data = std::fs::read(ca_cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
    let ca_certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&ca_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;
    if ca_certs.is_empty() {
        return Err(Error::Encoding("no CA certificates found".into()));
    }

    let mut root_store = rustls::RootCertStore::empty();
    for cert in ca_certs {
        root_store
            .add(cert)
            .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
    }

    let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
        .build()
        .map_err(|e| Error::Encoding(format!("failed to build client verifier: {e}")))?;

    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(certs, key)
        .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?;

    Ok(Arc::new(config))
}

/// Require explicit SC credential paths without performing file I/O.
pub fn required_sc_credentials<'a>(
    ca_cert_path: Option<&'a str>,
    client_cert_path: Option<&'a str>,
    client_key_path: Option<&'a str>,
) -> Result<[&'a str; 3], Error> {
    let required = |path: Option<&'a str>, name: &str| {
        path.filter(|path| !path.is_empty()).ok_or_else(|| {
            Error::Encoding(format!("{name} must be a nonempty path for SC mutual TLS"))
        })
    };
    Ok([
        required(ca_cert_path, "sc_ca_cert")?,
        required(client_cert_path, "sc_client_cert")?,
        required(client_key_path, "sc_client_key")?,
    ])
}

/// Build a TLS 1.3 client config using only explicit site trust and credentials.
pub fn build_client_tls_config(
    ca_cert_path: Option<&str>,
    client_cert_path: Option<&str>,
    client_key_path: Option<&str>,
) -> Result<Arc<tokio_rustls::rustls::ClientConfig>, Error> {
    use tokio_rustls::rustls;

    let [ca_path, cert_path, key_path] =
        required_sc_credentials(ca_cert_path, client_cert_path, client_key_path)?;
    let mut root_store = rustls::RootCertStore::empty();
    let ca_data = std::fs::read(ca_path)
        .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
    let ca_certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&ca_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;
    if ca_certs.is_empty() {
        return Err(Error::Encoding("no CA certificates found".into()));
    }
    for cert in ca_certs {
        root_store
            .add(cert)
            .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
    }

    let cert_data = std::fs::read(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read client cert: {e}")))?;
    let key_data = std::fs::read(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read client key: {e}")))?;
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse client cert: {e}")))?;
    if certs.is_empty() {
        return Err(Error::Encoding("no client certificates found".into()));
    }
    let key = PrivateKeyDer::from_pem_slice(&key_data)
        .map_err(|e| Error::Encoding(format!("failed to parse client key: {e}")))?;

    // Retain the local TLS-1.3-only policy; AB.7.4 requires TLS 1.3 support.
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)
        .map_err(|e| Error::Encoding(format!("TLS client auth error: {e}")))?;

    Ok(Arc::new(config))
}
