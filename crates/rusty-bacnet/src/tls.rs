//! TLS configuration helpers for Python bindings.

use bacnet_transport::sc_hub::ScHubTlsConfig;
use bacnet_transport::sc_tls::{ScNodeTlsConfig, pem};
use bacnet_types::error::Error;
use tokio_rustls::rustls::pki_types::CertificateDer;

/// Load Python hub credentials, then build the constrained native TLS policy.
pub fn build_server_tls_config(
    cert_path: &str,
    key_path: &str,
    ca_cert_path: &str,
) -> Result<ScHubTlsConfig, Error> {
    let cert_data = std::fs::read(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read server cert: {e}")))?;
    let key_data = std::fs::read(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read server key: {e}")))?;

    let certs: Vec<CertificateDer<'static>> = pem::certificates(&cert_data)
        .map_err(|e| Error::Encoding(format!("failed to parse server cert: {e}")))?;
    if certs.is_empty() {
        return Err(Error::Encoding("no server certificates found".into()));
    }
    let key = pem::private_key(&key_data)
        .map_err(|e| Error::Encoding(format!("failed to parse server key: {e}")))?;

    let ca_data = std::fs::read(ca_cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
    let ca_certs: Vec<CertificateDer<'static>> = pem::certificates(&ca_data)
        .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;
    ScHubTlsConfig::from_der(ca_certs, certs, key)
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
) -> Result<ScNodeTlsConfig, Error> {
    use tokio_rustls::rustls;

    let [ca_path, cert_path, key_path] =
        required_sc_credentials(ca_cert_path, client_cert_path, client_key_path)?;
    let mut root_store = rustls::RootCertStore::empty();
    let ca_data = std::fs::read(ca_path)
        .map_err(|e| Error::Encoding(format!("failed to read CA cert: {e}")))?;
    let ca_certs: Vec<CertificateDer<'static>> = pem::certificates(&ca_data)
        .map_err(|e| Error::Encoding(format!("failed to parse CA cert: {e}")))?;
    if ca_certs.is_empty() {
        return Err(Error::Encoding("no CA certificates found".into()));
    }
    // Retain CA validation before reading identity files, but construct the
    // actual connection policy only through the shared native factory below.
    for cert in &ca_certs {
        root_store
            .add(cert.clone())
            .map_err(|e| Error::Encoding(format!("failed to add CA cert: {e}")))?;
    }

    let cert_data = std::fs::read(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read client cert: {e}")))?;
    let key_data = std::fs::read(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read client key: {e}")))?;
    let certs: Vec<CertificateDer<'static>> = pem::certificates(&cert_data)
        .map_err(|e| Error::Encoding(format!("failed to parse client cert: {e}")))?;
    if certs.is_empty() {
        return Err(Error::Encoding("no client certificates found".into()));
    }
    let key = pem::private_key(&key_data)
        .map_err(|e| Error::Encoding(format!("failed to parse client key: {e}")))?;

    ScNodeTlsConfig::from_der(ca_certs, certs, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ca_batch_decode_precedes_der_validation_and_identity_file_reads() {
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = Temp(
            std::env::temp_dir().join(format!("sc-python-ca-order-{}.pem", std::process::id())),
        );
        let bad_der = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
        let bad_pem = "-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----\n";
        for contents in [format!("{bad_der}{bad_pem}"), format!("{bad_pem}{bad_der}")] {
            std::fs::write(&path.0, contents).unwrap();
            let error = build_client_tls_config(
                path.0.to_str(),
                Some("missing-identity.pem"),
                Some("missing-key.pem"),
            );
            assert!(
                error
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("failed to parse CA cert")
            );
        }
        std::fs::write(&path.0, bad_der).unwrap();
        let error = build_client_tls_config(
            path.0.to_str(),
            Some("missing-identity.pem"),
            Some("missing-key.pem"),
        );
        assert!(
            error
                .err()
                .unwrap()
                .to_string()
                .contains("failed to add CA cert")
        );
    }
}
