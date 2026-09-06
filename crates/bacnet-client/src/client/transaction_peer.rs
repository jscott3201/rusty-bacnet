use super::*;

pub(super) struct TransactionPeer {
    pub(super) tsm_mac: MacAddr,
    pub(super) canonical: CanonicalPeer,
}

impl ConfirmedTarget<'_> {
    pub(super) fn transaction_peer(self) -> TransactionPeer {
        match self {
            Self::Local { mac } => TransactionPeer {
                tsm_mac: MacAddr::from_slice(mac),
                canonical: CanonicalPeer::direct(mac),
            },
            Self::Routed {
                dest_network,
                dest_mac,
                ..
            } => TransactionPeer {
                tsm_mac: routed_tsm_mac(dest_network, dest_mac),
                canonical: CanonicalPeer::routed(dest_network, dest_mac),
            },
        }
    }
}

pub(super) fn response_transaction_peer(
    source_mac: &[u8],
    source_network: &Option<NpduAddress>,
) -> TransactionPeer {
    match source_network {
        Some(address) if !address.mac_address.is_empty() => TransactionPeer {
            tsm_mac: routed_tsm_mac(address.network, &address.mac_address),
            canonical: CanonicalPeer::routed(address.network, &address.mac_address),
        },
        _ => TransactionPeer {
            tsm_mac: MacAddr::from_slice(source_mac),
            canonical: CanonicalPeer::direct(source_mac),
        },
    }
}

fn routed_tsm_mac(network: u16, mac: &[u8]) -> MacAddr {
    let mut key = MacAddr::new();
    key.extend_from_slice(&[0xFF, b'R']);
    key.extend_from_slice(&network.to_be_bytes());
    key.push(mac.len() as u8);
    key.extend_from_slice(mac);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_and_inbound_transaction_identities_agree() {
        let direct = ConfirmedTarget::Local { mac: &[1, 2, 3] }.transaction_peer();
        let direct_response = response_transaction_peer(&[1, 2, 3], &None);
        assert_eq!(direct.tsm_mac, direct_response.tsm_mac);
        assert_eq!(direct.canonical, direct_response.canonical);

        let routed = ConfirmedTarget::Routed {
            router_mac: &[9],
            dest_network: 42,
            dest_mac: &[4, 5],
        }
        .transaction_peer();
        let routed_response = response_transaction_peer(
            &[8],
            &Some(NpduAddress {
                network: 42,
                mac_address: MacAddr::from_slice(&[4, 5]),
            }),
        );
        assert_eq!(routed.tsm_mac, routed_response.tsm_mac);
        assert_eq!(routed.canonical, routed_response.canonical);
    }

    #[test]
    fn empty_routed_source_falls_back_to_immediate_mac() {
        let identity = response_transaction_peer(
            &[7, 8],
            &Some(NpduAddress {
                network: 99,
                mac_address: MacAddr::new(),
            }),
        );
        assert_eq!(identity.tsm_mac, MacAddr::from_slice(&[7, 8]));
        assert_eq!(identity.canonical, CanonicalPeer::direct(&[7, 8]));
    }
}
