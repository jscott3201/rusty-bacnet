//! Owned observations, independent of TLS policy and runtime ownership.

use std::fmt::{self, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use rustls::ClientConnection;
use x509_parser::nom::Parser;
use x509_parser::prelude::*;

use super::ScSubjectAltName;

/// How TLS authenticated this physical connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScHandshakeKind {
    /// A full handshake (including one with a HelloRetryRequest).
    Full,
    /// Authentication was inherited from a resumed session. Certificates need
    /// not have been sent or validated again on this connection.
    Resumed,
    /// The TLS implementation did not supply the handshake kind.
    Unknown,
}

/// Bounded display metadata. Parsing is informational, never verification.
///
/// Names and OIDs are ASCII escaped, at most 1024 bytes per field. Of the
/// extensions, only the Subject Alternative Names are read. Dates are signed
/// Unix seconds, not a current-validity or revocation verdict. No
/// certificate text is in Debug.
///
/// Parsing doesn't verify, but parsing a verified certificate, such as the
/// leaf a server policy context returns, reads what its CA signed.
#[derive(Clone, PartialEq, Eq)]
pub struct ScCertificateMetadata {
    subject: String,
    issuer: String,
    subject_alt_names: Vec<ScSubjectAltName>,
    not_before_unix: i64,
    not_after_unix: i64,
    public_key_algorithm_oid: String,
    signature_algorithm_oid: String,
    text_truncated: bool,
}

impl ScCertificateMetadata {
    /// Escaped subject distinguished name; not an SC identity principal.
    pub fn subject(&self) -> &str {
        &self.subject
    }
    /// Escaped issuer distinguished name; not proof of issuer authorization.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    /// The DNS, URI, email and IP Subject Alternative Names, unescaped and
    /// in certificate order (#1965). Other name forms are left out, and a
    /// missing, repeated or malformed extension gives none, so a rule should
    /// allow on a name it finds rather than deny on one it misses. Match by
    /// exact equality; see [`ScSubjectAltName`] for what that means.
    pub fn subject_alt_names(&self) -> &[ScSubjectAltName] {
        &self.subject_alt_names
    }
    /// Certificate not-before, in Unix seconds.
    pub fn not_before_unix(&self) -> i64 {
        self.not_before_unix
    }
    /// Certificate not-after, in Unix seconds.
    pub fn not_after_unix(&self) -> i64 {
        self.not_after_unix
    }
    /// Subject public-key algorithm OID (not the signature algorithm).
    pub fn public_key_algorithm_oid(&self) -> &str {
        &self.public_key_algorithm_oid
    }
    /// Certificate signature algorithm OID.
    pub fn signature_algorithm_oid(&self) -> &str {
        &self.signature_algorithm_oid
    }
    /// At least one display field exceeded its 1024-byte bound.
    pub fn text_truncated(&self) -> bool {
        self.text_truncated
    }
}

impl fmt::Debug for ScCertificateMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScCertificateMetadata")
            .field("text_truncated", &self.text_truncated)
            .finish_non_exhaustive()
    }
}

/// One original DER certificate with optional display interpretation.
/// A parser failure preserves the DER and does not change TLS acceptance.
#[derive(Clone, PartialEq, Eq)]
pub struct ScPeerCertificate {
    der: Vec<u8>,
    metadata: Option<ScCertificateMetadata>,
}

impl ScPeerCertificate {
    pub(super) fn new(der: &[u8]) -> Self {
        Self::from_owned_der(der.to_vec())
    }
    pub(super) fn from_owned_der(der: Vec<u8>) -> Self {
        let metadata = inspect(&der);
        Self { der, metadata }
    }
    /// Explicit export of the original certificate bytes, without re-encoding.
    pub fn der(&self) -> &[u8] {
        &self.der
    }
    /// Display interpretation, or None if the parser could not consume exactly
    /// one certificate. This is not a certificate verification result.
    pub fn metadata(&self) -> Option<&ScCertificateMetadata> {
        self.metadata.as_ref()
    }
}

impl fmt::Debug for ScPeerCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScPeerCertificate")
            .field("der_len", &self.der.len())
            .field("metadata_available", &self.metadata.is_some())
            .finish()
    }
}

/// Immutable session-associated peer certificates and physical connection data.
///
/// The list is ordered as supplied by rustls, leaf first; it may omit anchors
/// or contain certificates outside the validated path. Resumption uses cached
/// peer certificates. The locked TLS implementation bounds the certificate
/// message to 64 KiB. This object owns only data: retaining it keeps no socket,
/// TLS configuration, client or shutdown task alive. Debug omits peer data.
#[derive(Clone)]
pub struct ScTlsPeerInfo {
    connection_id: u64,
    peer_address: SocketAddr,
    server_name: String,
    handshake_kind: ScHandshakeKind,
    protocol_version: Option<String>,
    cipher_suite: Option<String>,
    leaf_sha256: [u8; 32],
    certificates: Vec<ScPeerCertificate>,
}

impl ScTlsPeerInfo {
    pub(super) fn capture(
        tls: &ClientConnection,
        peer_address: SocketAddr,
        server_name: String,
        leaf_sha256: [u8; 32],
    ) -> Self {
        // Process-local physical connection identity; independent of SC retry,
        // endpoint, certificate identity, and TLS session resumption.
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            connection_id: NEXT.fetch_add(1, Ordering::Relaxed),
            peer_address,
            server_name,
            handshake_kind: match tls.handshake_kind() {
                Some(
                    rustls::HandshakeKind::Full | rustls::HandshakeKind::FullWithHelloRetryRequest,
                ) => ScHandshakeKind::Full,
                Some(rustls::HandshakeKind::Resumed) => ScHandshakeKind::Resumed,
                _ => ScHandshakeKind::Unknown,
            },
            protocol_version: tls.protocol_version().map(|v| format!("{v:?}")),
            cipher_suite: tls
                .negotiated_cipher_suite()
                .map(|v| format!("{:?}", v.suite())),
            leaf_sha256,
            certificates: tls
                .peer_certificates()
                .unwrap_or_default()
                .iter()
                .map(|der| ScPeerCertificate::new(der.as_ref()))
                .collect(),
        }
    }
    /// Opaque process-local physical connection identifier. Fresh dials,
    /// including resumed TLS, get new values; same-socket SC retries do not.
    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }
    /// Actual remote TCP endpoint, which may be a proxy.
    pub fn peer_address(&self) -> SocketAddr {
        self.peer_address
    }
    /// URI host used as the intended TLS server name (without a port).
    pub fn server_name(&self) -> &str {
        &self.server_name
    }
    /// Full/resumed authentication provenance.
    pub fn handshake_kind(&self) -> ScHandshakeKind {
        self.handshake_kind
    }
    /// Negotiated TLS version as named by rustls, if available.
    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }
    /// Negotiated cipher suite as named by rustls, if available.
    pub fn cipher_suite(&self) -> Option<&str> {
        self.cipher_suite.as_deref()
    }
    /// SHA-256 of the exact leaf DER, the same digest used for leaf bindings.
    pub fn leaf_sha256(&self) -> &[u8; 32] {
        &self.leaf_sha256
    }
    /// Original session-associated peer certificate list, in original order.
    pub fn certificates(&self) -> &[ScPeerCertificate] {
        &self.certificates
    }
}

impl fmt::Debug for ScTlsPeerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScTlsPeerInfo")
            .field("connection_id", &self.connection_id)
            .field("handshake_kind", &self.handshake_kind)
            .field("certificate_count", &self.certificates.len())
            .finish_non_exhaustive()
    }
}

fn inspect(der: &[u8]) -> Option<ScCertificateMetadata> {
    let (remaining, cert) = X509CertificateParser::new()
        .with_deep_parse_extensions(false)
        .parse(der)
        .ok()?;
    if !remaining.is_empty() {
        return None;
    }
    Some(metadata(&cert))
}

pub(super) fn metadata(cert: &X509Certificate<'_>) -> ScCertificateMetadata {
    let mut truncated = false;
    ScCertificateMetadata {
        subject: escaped(&cert.subject(), &mut truncated),
        issuer: escaped(&cert.issuer(), &mut truncated),
        subject_alt_names: super::subject_alt_name::subject_alt_names(cert),
        not_before_unix: cert.validity().not_before.timestamp(),
        not_after_unix: cert.validity().not_after.timestamp(),
        public_key_algorithm_oid: escaped(&cert.public_key().algorithm.algorithm, &mut truncated),
        signature_algorithm_oid: escaped(&cert.signature_algorithm.algorithm, &mut truncated),
        text_truncated: truncated,
    }
}

// A bounded formatting sink avoids allocating an arbitrarily long Display
// result first. Escape controls and non-ASCII, including bidi control text.
pub(super) fn escaped(value: &impl fmt::Display, truncated: &mut bool) -> String {
    escaped_bounded(value, truncated, 1024)
}

pub(super) fn escaped_bounded(
    value: &impl fmt::Display,
    truncated: &mut bool,
    limit: usize,
) -> String {
    struct Escaped {
        value: String,
        truncated: bool,
        limit: usize,
    }
    impl Write for Escaped {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for c in s.chars() {
                let escaped = c.escape_default();
                if self.value.len() + escaped.len() > self.limit {
                    self.truncated = true;
                    return Err(fmt::Error);
                }
                self.value.extend(escaped);
            }
            Ok(())
        }
    }
    let mut output = Escaped {
        value: String::new(),
        truncated: false,
        limit,
    };
    let _ = write!(&mut output, "{value}");
    *truncated |= output.truncated;
    output.value
}

#[cfg(test)]
#[path = "peer_info_tests.rs"]
mod tests;
