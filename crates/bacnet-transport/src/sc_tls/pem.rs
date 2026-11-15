//! PEM decoding for the offline inspection, without TLS policy or its
//! limits. The TLS configurations decode through [`super::material`], which
//! is stricter about the private key.
//!
//! Unknown sections are skipped. Certificates retain input order. Private-key
//! decoding returns the first recognized unencrypted PKCS#1, SEC1 or PKCS#8
//! key and deliberately does not inspect the remaining input. These functions
//! preserve Rustls's PEM error values; callers choose their error presentation.

use rustls::pki_types::pem::{Error, PemObject, SliceIter};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Iterate certificates without decoding later items ahead of the caller.
pub(super) fn certificate_iter(pem: &[u8]) -> SliceIter<'_, CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem)
}

/// Decode the first recognized unencrypted private key.
pub(super) fn private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, Error> {
    PrivateKeyDer::from_pem_slice(pem)
}

#[cfg(test)]
#[path = "pem_tests.rs"]
mod tests;
