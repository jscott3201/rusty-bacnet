//! Shared PEM decoding, without TLS policy or offline-inspection limits.
//!
//! Unknown sections are skipped. Certificates retain input order. Private-key
//! decoding returns the first recognized unencrypted PKCS#1, SEC1 or PKCS#8
//! key and deliberately does not inspect the remaining input. These functions
//! preserve Rustls's PEM error values; callers choose their error presentation.

use std::io::{BufReader, Read};

use rustls::pki_types::pem::{Error, PemObject, ReadIter, SliceIter};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Iterate certificates without decoding later items ahead of the caller.
pub fn certificate_iter(pem: &[u8]) -> SliceIter<'_, CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem)
}

/// Iterate certificates from a caller-owned reader. Opening files remains the
/// caller's responsibility, preserving its I/O and error ordering.
pub fn certificate_reader<R: Read>(reader: R) -> ReadIter<BufReader<R>, CertificateDer<'static>> {
    CertificateDer::pem_reader_iter(reader)
}

/// Eagerly decode all certificates. An empty bundle yields an empty vector.
pub fn certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, Error> {
    certificate_iter(pem).collect()
}

/// Decode the first recognized unencrypted private key.
pub fn private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, Error> {
    PrivateKeyDer::from_pem_slice(pem)
}

/// Decode the first recognized private key from a caller-owned reader.
pub fn private_key_reader(reader: impl Read) -> Result<PrivateKeyDer<'static>, Error> {
    PrivateKeyDer::from_pem_reader(reader)
}
