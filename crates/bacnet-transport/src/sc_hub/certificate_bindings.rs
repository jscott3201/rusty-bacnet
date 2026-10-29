//! Opt-in installation policy; configuration pins are never connection identity.

use crate::sc_frame::{BROADCAST_VMAC, UNKNOWN_VMAC, Vmac};
use bacnet_types::error::Error;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

/// One immutable certificate rotation group with a provisioned UUID and VMACs.
/// Digests are SHA-256 of the exact operational leaf DER, not PEM or its key.
/// This is installation policy, not end-to-end authentication of relayed traffic.
#[derive(Clone)]
pub struct ScHubCertificateBinding {
    uuid: [u8; 16],
    allowed_vmacs: Vec<Vmac>,
    leaf_sha256: Vec<[u8; 32]>,
}

impl ScHubCertificateBinding {
    /// Validate owned configuration. UUID must be nonzero; both lists must be
    /// nonempty and distinct; UNKNOWN/BROADCAST VMACs are forbidden.
    pub fn new(
        uuid: [u8; 16],
        allowed_vmacs: Vec<Vmac>,
        leaf_sha256: Vec<[u8; 32]>,
    ) -> Result<Self, Error> {
        if uuid == [0; 16] {
            return Err(invalid("UUID must not be zero"));
        }
        if allowed_vmacs.is_empty() || leaf_sha256.is_empty() {
            return Err(invalid("VMAC and leaf lists must be nonempty"));
        }
        let mut seen = HashSet::new();
        for vmac in &allowed_vmacs {
            if *vmac == UNKNOWN_VMAC || *vmac == BROADCAST_VMAC {
                return Err(invalid("reserved VMAC"));
            }
            if !seen.insert(*vmac) {
                return Err(invalid("duplicate VMAC"));
            }
        }
        let mut seen = HashSet::new();
        if leaf_sha256.iter().any(|digest| !seen.insert(*digest)) {
            return Err(invalid("duplicate leaf digest"));
        }
        Ok(Self {
            uuid,
            allowed_vmacs,
            leaf_sha256,
        })
    }
    /// Provisioned identity; not a verified connection identity.
    pub fn uuid(&self) -> [u8; 16] {
        self.uuid
    }
    /// Allowed port claims for this provisioned identity.
    pub fn allowed_vmacs(&self) -> &[Vmac] {
        &self.allowed_vmacs
    }
    /// Explicitly authorized current/renewal leaf digests.
    pub fn leaf_sha256(&self) -> &[[u8; 32]] {
        &self.leaf_sha256
    }
}
impl fmt::Debug for ScHubCertificateBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScHubCertificateBinding")
            .finish_non_exhaustive()
    }
}

/// Nonempty, immutable mapped-only admission policy. All configured UUIDs,
/// VMACs and fingerprints are reserved even while their owners are offline.
/// Clones share policy, never live membership or counters.
#[derive(Clone)]
pub struct ScHubCertificateBindings(Arc<BindingMap>);
struct BindingMap {
    groups: Vec<ScHubCertificateBinding>,
    by_leaf: HashMap<[u8; 32], usize>,
    vmacs: HashSet<Vmac>,
}
impl ScHubCertificateBindings {
    /// Reject empty input or overlapping UUID, VMAC or digest ownership.
    pub fn new(groups: Vec<ScHubCertificateBinding>) -> Result<Self, Error> {
        if groups.is_empty() {
            return Err(invalid("map must be nonempty"));
        }
        let mut uuids = HashSet::new();
        let mut vmacs = HashSet::new();
        let mut by_leaf = HashMap::new();
        for (index, group) in groups.iter().enumerate() {
            if !uuids.insert(group.uuid) {
                return Err(invalid("overlapping UUID ownership"));
            }
            for vmac in &group.allowed_vmacs {
                if !vmacs.insert(*vmac) {
                    return Err(invalid("overlapping VMAC ownership"));
                }
            }
            for leaf in &group.leaf_sha256 {
                if by_leaf.insert(*leaf, index).is_some() {
                    return Err(invalid("overlapping leaf ownership"));
                }
            }
        }
        Ok(Self(Arc::new(BindingMap {
            groups,
            by_leaf,
            vmacs,
        })))
    }
    /// Validate the hosting port before credential I/O or bind. A group UUID
    /// equal to the Hub UUID is not forbidden; UUID is not the local port VMAC.
    pub fn validate_hub_vmac(&self, hub_vmac: Vmac) -> Result<(), Error> {
        if self.0.vmacs.contains(&hub_vmac) {
            Err(invalid("allowed VMAC overlaps local Hub VMAC"))
        } else {
            Ok(())
        }
    }
    pub(super) fn permits(&self, leaf: Option<&VerifiedLeaf>, uuid: [u8; 16], vmac: Vmac) -> bool {
        let Some(group) = leaf
            .and_then(|leaf| self.0.by_leaf.get(&leaf.0))
            .map(|index| &self.0.groups[*index])
        else {
            return false;
        };
        group.uuid == uuid && group.allowed_vmacs.contains(&vmac)
    }
}
impl fmt::Debug for ScHubCertificateBindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScHubCertificateBindings")
            .finish_non_exhaustive()
    }
}
fn invalid(reason: &str) -> Error {
    Error::Encoding(format!("certificate bindings: {reason}"))
}

/// Minted only from rustls's peer chain after a successful verifying handshake.
/// Never exposed to callers or constructed from Connect claims.
pub(super) struct VerifiedLeaf([u8; 32]);
impl VerifiedLeaf {
    pub(super) fn from_verified_chain(
        chain: Option<&[rustls::pki_types::CertificateDer<'_>]>,
    ) -> Option<Self> {
        let leaf = chain?.first()?;
        Some(Self(
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, leaf.as_ref())
                .as_ref()
                .try_into()
                .expect("SHA-256 output length"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificate_bindings_missing_identity_fails_closed_and_hashes_exact_leaf() {
        assert!(VerifiedLeaf::from_verified_chain(None).is_none());
        assert!(VerifiedLeaf::from_verified_chain(Some(&[])).is_none());
        // Independent FIPS180-4 SHA256("abc") known answer; trailing chain
        // entries must never participate in the leaf fingerprint.
        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        let chain = [
            rustls::pki_types::CertificateDer::from(b"abc".to_vec()),
            rustls::pki_types::CertificateDer::from(b"ignored issuer".to_vec()),
        ];
        let leaf = VerifiedLeaf::from_verified_chain(Some(&chain)).unwrap();
        let map = ScHubCertificateBindings::new(vec![
            ScHubCertificateBinding::new([1; 16], vec![[1; 6]], vec![expected]).unwrap(),
        ])
        .unwrap();
        assert!(!map.permits(None, [1; 16], [1; 6]));
        assert!(map.permits(Some(&leaf), [1; 16], [1; 6]));
        assert!(!map.permits(Some(&leaf), [2; 16], [1; 6]));
    }
}
