//! The certificate principal of a verified direct BACnet/SC peer (#524, #518).

use std::fmt;

/// Who sent a request, as far as this node can prove it: the leaf
/// certificate of a verified direct BACnet/SC connection.
///
/// It is the SHA-256 of the exact leaf certificate DER the TLS handshake
/// verified, the same digest the hub's certificate bindings pin
/// (`ScHubCertificateBinding`, with the `sc-tls` feature).
/// Unlike [`DirectScIdentity`](crate::port::DirectScIdentity), it carries no
/// connection incarnation, so it stays the same when that certificate
/// reconnects. A renewed certificate is a new principal; list both while it
/// rolls out.
///
/// Only direct SC ingress has one. Requests over B/IP, B/IPv6, MS/TP and
/// Ethernet have none, and neither do requests a hub relays: the hub
/// authenticated its own TLS client, and the origin's certificate never
/// reaches this node. The VMAC, UUID and routed SNET/SADR a peer sends are
/// claims, never principals. A direct peer that forwards a routed request is
/// only the router: the server's policy contexts report its principal as the
/// forwarding peer's and give the routed request none.
///
/// A principal identifies; it doesn't authorize. Compare it against the
/// fingerprints an installation provisions. Building one with
/// [`from_leaf_sha256`](Self::from_leaf_sha256) makes such a pin; it can't
/// make a request carry it.
///
/// `Debug` shows only the first eight hex digits; `Display` writes all 64
/// in lowercase, the form to copy into configuration. Neither
/// shows any other certificate content.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScPrincipal {
    leaf_sha256: [u8; 32],
}

impl ScPrincipal {
    /// The principal whose leaf certificate DER has SHA-256 `leaf_sha256`,
    /// for comparing against the principal a request carries.
    pub const fn from_leaf_sha256(leaf_sha256: [u8; 32]) -> Self {
        Self { leaf_sha256 }
    }

    /// SHA-256 of the leaf certificate DER.
    pub const fn leaf_sha256(&self) -> [u8; 32] {
        self.leaf_sha256
    }
}

impl fmt::Display for ScPrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.leaf_sha256
            .iter()
            .try_for_each(|octet| write!(f, "{octet:02x}"))
    }
}

impl fmt::Debug for ScPrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScPrincipal(sha256:")?;
        self.leaf_sha256[..4]
            .iter()
            .try_for_each(|octet| write!(f, "{octet:02x}"))?;
        write!(f, "..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_full_hex_and_debug_is_a_prefix() {
        let mut digest = [0u8; 32];
        digest[0] = 0xab;
        digest[31] = 0x01;
        let principal = ScPrincipal::from_leaf_sha256(digest);
        let hex = principal.to_string();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab000000") && hex.ends_with("01"));
        assert_eq!(format!("{principal:?}"), "ScPrincipal(sha256:ab000000..)");
        assert_eq!(principal.leaf_sha256(), digest);
    }
}
