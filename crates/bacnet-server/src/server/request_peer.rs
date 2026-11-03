use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;

/// Logical inbound source, not an authenticated principal (including on SC).
/// Shared only by exact duplicate detection and top-level request admission.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) enum CanonicalRequester {
    Direct(MacAddr),
    Routed(NpduAddress),
}

// By hand: a derived clone copies each address a byte at a time, and request
// admission clones the requester for every confirmed request (#1978).
impl Clone for CanonicalRequester {
    fn clone(&self) -> Self {
        match self {
            Self::Direct(mac) => Self::Direct(MacAddr::from_slice(mac)),
            Self::Routed(source) => Self::Routed(copy_address(source)),
        }
    }
}

fn copy_address(source: &NpduAddress) -> NpduAddress {
    NpduAddress {
        network: source.network,
        mac_address: MacAddr::from_slice(&source.mac_address),
    }
}

pub(super) fn canonical_requester(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
) -> CanonicalRequester {
    source_network
        .filter(|source| (1..=0xfffe).contains(&source.network) && !source.mac_address.is_empty())
        .map(|source| CanonicalRequester::Routed(copy_address(source)))
        .unwrap_or_else(|| CanonicalRequester::Direct(MacAddr::from_slice(source_mac)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copied requester keeps the variant, the network and every address
    /// length, inline or spilled, and owns its bytes (#1978: `Clone` is by
    /// hand).
    #[test]
    fn copied_requesters_preserve_address_network_variant_and_ownership() {
        let mut identities = std::collections::HashMap::new();
        for len in [0, 1, 3, 6, 7, 18, 23, 64] {
            let bytes: Vec<u8> = (0..len).map(|index| index as u8).collect();
            let routed = |network| {
                CanonicalRequester::Routed(NpduAddress {
                    network,
                    mac_address: MacAddr::from_slice(&bytes),
                })
            };
            for mut original in [
                CanonicalRequester::Direct(MacAddr::from_slice(&bytes)),
                routed(42),
                routed(43),
            ] {
                let copied = original.clone();
                assert_eq!(copied, original);
                assert!(identities.insert(copied.clone(), len).is_none());
                let (original_mac, copied_mac) = match (&mut original, &copied) {
                    (CanonicalRequester::Direct(a), CanonicalRequester::Direct(b)) => (a, b),
                    (CanonicalRequester::Routed(a), CanonicalRequester::Routed(b)) => {
                        (&mut a.mac_address, &b.mac_address)
                    }
                    _ => unreachable!(),
                };
                assert_eq!(copied_mac.as_slice(), bytes);
                assert_eq!(copied_mac.spilled(), len > 6);
                original_mac.push(0xff);
                assert_ne!(original, copied);
                assert_eq!(identities.get(&copied), Some(&len));
                assert!(!identities.contains_key(&original));
            }
        }
        assert_eq!(identities.len(), 24);
    }

    /// A valid routed source is copied whole; any other falls back to the
    /// link MAC.
    #[test]
    fn routed_requesters_copy_the_source_address() {
        let source = NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[1, 2, 3, 4, 5, 6, 7]),
        };
        assert_eq!(
            canonical_requester(&[9], Some(&source)),
            CanonicalRequester::Routed(source.clone())
        );
        for network in [0, 0xffff] {
            let source = NpduAddress {
                network,
                ..source.clone()
            };
            assert_eq!(
                canonical_requester(&[9], Some(&source)),
                CanonicalRequester::Direct(MacAddr::from_slice(&[9]))
            );
        }
    }
}
