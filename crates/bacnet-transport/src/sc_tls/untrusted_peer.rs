//! Owned evidence from an unsuccessful outbound TLS attempt.

use std::{fmt, net::SocketAddr};

use super::ScPeerCertificate;

/// Untrusted certificates observed during one failed outbound TLS handshake.
///
/// This is diagnostic input from the peer, **not** an authenticated identity,
/// accepted SC connection, verified chain, or revocation verdict. The list is
/// leaf first as presented to the normal verifier, including any unused extra
/// certificates. Metadata parsing never changes the original TLS result.
///
/// Created only when certificate verification was reached. Failures before
/// that observation point and resumed handshakes can have no such evidence;
/// absence does not establish that the peer sent no certificate. Retention
/// owns data only, without a connection, TLS configuration, or background task.
#[derive(Clone, PartialEq, Eq)]
pub struct ScUntrustedPeerInfo {
    peer_address: SocketAddr,
    server_name: String,
    certificates: Vec<ScPeerCertificate>,
}

impl ScUntrustedPeerInfo {
    pub(super) fn new(
        peer_address: SocketAddr,
        server_name: String,
        certificates: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            peer_address,
            server_name,
            certificates: certificates
                .into_iter()
                .map(ScPeerCertificate::from_owned_der)
                .collect(),
        }
    }

    /// TCP endpoint of the failed attempt, not an authenticated peer identity.
    pub fn peer_address(&self) -> SocketAddr {
        self.peer_address
    }

    /// URI host supplied to verification, not an accepted certificate name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Original untrusted peer certificate bytes with optional display metadata.
    pub fn certificates(&self) -> &[ScPeerCertificate] {
        &self.certificates
    }
}

impl fmt::Debug for ScUntrustedPeerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScUntrustedPeerInfo")
            .field("certificate_count", &self.certificates.len())
            .finish_non_exhaustive()
    }
}
