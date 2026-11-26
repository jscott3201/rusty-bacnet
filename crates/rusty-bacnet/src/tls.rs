//! TLS configuration helpers for Python bindings: the native PEM
//! constructors (#2600), with their errors as the bindings report them.

use bacnet_transport::sc_hub::ScHubTlsConfig;
use bacnet_transport::sc_tls::ScNodeTlsConfig;
use bacnet_types::error::{Error, Refusal};

/// Load Python hub credentials into the constrained native TLS policy: the
/// CA bundle first, then the hub's chain and key.
pub fn build_server_tls_config(
    cert_path: &str,
    key_path: &str,
    ca_cert_path: &str,
) -> Result<ScHubTlsConfig, Error> {
    Ok(ScHubTlsConfig::from_pem_files(
        ca_cert_path,
        cert_path,
        key_path,
    )?)
}

/// Require explicit SC credential paths without performing file I/O.
pub fn required_sc_credentials<'a>(
    ca_cert_path: Option<&'a str>,
    client_cert_path: Option<&'a str>,
    client_key_path: Option<&'a str>,
) -> Result<[&'a str; 3], Error> {
    let required = |path: Option<&'a str>, name: &str| {
        path.filter(|path| !path.is_empty()).ok_or_else(|| {
            Error::InvalidConfig(Refusal::text(format!(
                "{name} must be a nonempty path for SC mutual TLS"
            )))
        })
    };
    Ok([
        required(ca_cert_path, "sc_ca_cert")?,
        required(client_cert_path, "sc_client_cert")?,
        required(client_key_path, "sc_client_key")?,
    ])
}

/// Build a TLS 1.3 client config using only explicit site trust and
/// credentials: the CA bundle is read and checked before the identity files.
pub fn build_client_tls_config(
    ca_cert_path: Option<&str>,
    client_cert_path: Option<&str>,
    client_key_path: Option<&str>,
) -> Result<ScNodeTlsConfig, Error> {
    let [ca_path, cert_path, key_path] =
        required_sc_credentials(ca_cert_path, client_cert_path, client_key_path)?;
    Ok(ScNodeTlsConfig::from_pem_files(
        ca_path, cert_path, key_path,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CA file is checked block by block, before the identity files
    /// are read: the first fault in it is the one reported.
    #[test]
    fn node_ca_faults_precede_identity_file_reads_in_file_order() {
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
        for (contents, expected) in [
            (
                format!("{bad_der}{bad_pem}"),
                "certificate 0 of the CA bundle can't be used",
            ),
            (
                format!("{bad_pem}{bad_der}"),
                "the CA bundle is not valid PEM",
            ),
            (
                bad_der.to_owned(),
                "certificate 0 of the CA bundle can't be used",
            ),
        ] {
            std::fs::write(&path.0, contents).unwrap();
            let error = build_client_tls_config(
                path.0.to_str(),
                Some("missing-identity.pem"),
                Some("missing-key.pem"),
            )
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
}
