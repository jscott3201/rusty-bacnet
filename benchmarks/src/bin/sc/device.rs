//! Private standalone-device policy; public raw Rust TLS APIs remain unchanged.

use super::credentials::{required, Credentials};
use super::Args;
use std::sync::Arc;

pub(super) struct ScConfig<'a> {
    pub url: &'a str,
    pub vmac: [u8; 6],
    pub uuid: [u8; 16],
    pub tls: Arc<rustls::ClientConfig>,
}

impl<'a> ScConfig<'a> {
    pub fn load(args: &'a Args) -> Result<Self, String> {
        // Check every required input and identity before reading any local file.
        let url = required(args.sc_hub.as_deref(), "--sc-hub")?;
        let ca = required(args.sc_ca.as_deref(), "--sc-ca")?;
        let cert = required(args.sc_cert.as_deref(), "--sc-cert")?;
        let key = required(args.sc_key.as_deref(), "--sc-key")?;
        let vmac = hex(required(args.sc_vmac.as_deref(), "--sc-vmac")?, "--sc-vmac")?;
        if vmac == [0; 6] || vmac == [255; 6] {
            return Err("--sc-vmac must not be unknown (all zero) or broadcast (all ff)".into());
        }
        let uuid = hex(
            required(args.sc_device_uuid.as_deref(), "--sc-device-uuid")?,
            "--sc-device-uuid",
        )?;
        if uuid == [0; 16] {
            return Err("--sc-device-uuid must be nonzero and unique on this SC network".into());
        }
        let material = Credentials::load(ca, cert, key, ["--sc-ca", "--sc-cert", "--sc-key"])?;
        let mut roots = rustls::RootCertStore::empty();
        for cert in material.ca {
            roots
                .add(cert)
                .map_err(|e| format!("--sc-ca: invalid CA certificate: {e}"))?;
        }
        for cert in &material.chain {
            rustls::server::ParsedCertificate::try_from(cert)
                .map_err(|e| format!("--sc-cert: invalid certificate DER: {e}"))?;
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        // with_client_auth_cert validates the built-in provider's signing key
        // against the leaf before TlsWebSocket can perform DNS or TCP I/O.
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| format!("SC TLS configuration: {e}"))?
            .with_root_certificates(roots)
            .with_client_auth_cert(material.chain, material.key)
            .map_err(|e| format!("--sc-cert/--sc-key: invalid or mismatched credentials: {e}"))?;
        Ok(Self {
            url,
            vmac,
            uuid,
            tls: Arc::new(tls),
        })
    }
}

fn hex<const N: usize>(text: &str, flag: &str) -> Result<[u8; N], String> {
    // Explicit fixed-length ASCII avoids byte slicing arbitrary UTF-8 input.
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "{flag}: expected {} hexadecimal digits without separators",
            N * 2
        ));
    }
    let mut bytes = [0; N];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b.to_ascii_lowercase() - b'a' + 10
            }
        };
        *byte = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(bytes)
}
