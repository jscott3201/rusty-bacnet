//! Private file-loading boundary shared only by the two standalone binaries.

use bacnet_transport::sc_tls::pem;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub(super) fn required<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a str, String> {
    value.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        format!("{flag} is required and must be nonempty; see examples/docker/README.md")
    })
}

pub(super) struct Credentials {
    pub ca: Vec<CertificateDer<'static>>,
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl Credentials {
    pub fn load(ca: &str, cert: &str, key: &str, flags: [&str; 3]) -> Result<Self, String> {
        let ca = certificates(ca, flags[0])?;
        let chain = certificates(cert, flags[1])?;
        let key = pem::private_key(&read(key, flags[2])?).map_err(|_| {
            format!(
                "{}: expected a usable unencrypted PEM private key",
                flags[2]
            )
        })?;
        Ok(Self { ca, chain, key })
    }
}

fn read(path: &str, flag: &str) -> Result<Vec<u8>, String> {
    // Do not echo supplied contents (or a malicious path) in diagnostics.
    std::fs::read(path).map_err(|e| format!("{flag}: cannot read PEM file ({})", e.kind()))
}

fn certificates(path: &str, flag: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = pem::certificates(&read(path, flag)?)
        .map_err(|_| format!("{flag}: malformed certificate PEM"))?;
    if certs.is_empty() {
        return Err(format!("{flag}: no PEM certificates found"));
    }
    Ok(certs)
}
