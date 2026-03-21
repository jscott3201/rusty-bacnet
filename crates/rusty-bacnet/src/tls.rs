//! TLS configuration helpers for Python bindings.

use bacnet_types::error::Error;
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Build a rustls ServerConfig from PEM file paths (for ScHub).
pub fn build_server_tls_config(
    cert_path: &str,
    key_path: &str,
    ca_cert_path: Option<&str>,
) -> Result<Arc<tokio_rustls::rustls::ServerConfig>, Error> {
    use tokio_rustls::rustls;

    let cert_data = std::fs::read(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read server cert: {e}")))?;
    let key_data = std::fs::read(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read server key: {e}")))?;

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse server cert: {e}")))?;
    let key = PrivateKeyDer::from_pem_slice(&key_data)
        .map_err(|e| Error::Encoding(format!("failed to parse server key: {e}")))?;

    let config = if let Some(ca_path) = ca_cert_path {
        // mTLS: require client certificates
        let ca_data = std::fs::read(ca_path)
            .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
        let ca_certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&ca_data)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;

        let mut root_store = rustls::RootCertStore::empty();
        for cert in ca_certs {
            root_store
                .add(cert)
                .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
        }

        let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .map_err(|e| Error::Encoding(format!("failed to build client verifier: {e}")))?;

        rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?
    } else {
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| Error::Encoding(format!("TLS server config error: {e}")))?
    };

    Ok(Arc::new(config))
}

/// Build a rustls ClientConfig from optional PEM file paths.
pub fn build_client_tls_config(
    ca_cert_path: Option<&str>,
    client_cert_path: Option<&str>,
    client_key_path: Option<&str>,
) -> Result<Arc<tokio_rustls::rustls::ClientConfig>, Error> {
    use tokio_rustls::rustls;

    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ca_path) = ca_cert_path {
        let ca_data = std::fs::read(ca_path)
            .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&ca_data)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;
        for cert in certs {
            root_store
                .add(cert)
                .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
        }
    } else {
        // Use system roots as fallback
        let native_result = rustls_native_certs::load_native_certs();
        if !native_result.errors.is_empty() {
            eprintln!(
                "Warning: some native TLS certificates could not be loaded: {:?}",
                native_result.errors
            );
        }
        if native_result.certs.is_empty() {
            return Err(Error::Encoding(
                "no native CA certificates could be loaded — provide ca_cert_path explicitly"
                    .into(),
            ));
        }
        for cert in native_result.certs {
            let _ = root_store.add(cert);
        }
    }

    // BACnet/SC requires TLS 1.3 per spec AB.7.4
    let builder = rustls::ClientConfig::builder_with_protocol_versions(&[
            &rustls::version::TLS13,
        ])
        .with_root_certificates(root_store);

    let config = match (client_cert_path, client_key_path) {
        (Some(cert_path), Some(key_path)) => {
            let cert_data = std::fs::read(cert_path)
                .map_err(|e| Error::Encoding(format!("failed to read client cert: {e}")))?;
            let key_data = std::fs::read(key_path)
                .map_err(|e| Error::Encoding(format!("failed to read client key: {e}")))?;

            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_data)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::Encoding(format!("failed to parse client cert: {e}")))?;

            let key = PrivateKeyDer::from_pem_slice(&key_data)
                .map_err(|e| Error::Encoding(format!("failed to parse client key: {e}")))?;

            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| Error::Encoding(format!("TLS client auth error: {e}")))?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => {
            return Err(Error::Encoding(
                "sc_client_cert and sc_client_key must both be provided or both omitted".into(),
            ));
        }
    };

    Ok(Arc::new(config))
}
