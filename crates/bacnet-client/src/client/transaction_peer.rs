use super::*;
use bacnet_endpoint_core::coordinator::response_invoke_id;

/// The peer a confirmed transaction is keyed to: its canonical identity and
/// the TSM key derived from it. The standalone client and the endpoint
/// requester both key requests and match answers through here, so the two
/// sides of a transaction always agree.
pub(crate) struct TransactionPeer {
    pub(crate) tsm_mac: MacAddr,
    pub(crate) canonical: CanonicalPeer,
}

impl TransactionPeer {
    /// The transaction peer for `canonical`. A direct peer's TSM key is its
    /// MAC. A routed peer's is the `FF 52` synthetic key (`network || len ||
    /// address`), so it never collides with a direct peer that happens to
    /// share trailing bytes.
    pub(crate) fn of(canonical: CanonicalPeer) -> Self {
        // `from_slice` copies the bytes at once; `clone` would copy them one
        // at a time (#1978).
        let tsm_mac = match &canonical {
            CanonicalPeer::Direct(mac) => MacAddr::from_slice(mac),
            CanonicalPeer::Routed { network, address } => routed_tsm_mac(*network, address),
        };
        Self { tsm_mac, canonical }
    }

    /// The transaction an answer belongs to, from the answer's link MAC and
    /// SNET/SADR. With `local_network` known, an answer a router relays back
    /// with this network as its SNET belongs to the same transaction as one
    /// sent straight from its SADR (#1465); see [`CanonicalPeer::from_source`].
    pub(crate) fn of_answer(
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        local_network: Option<u16>,
    ) -> Self {
        Self::of(CanonicalPeer::from_source(
            source_mac,
            source_network,
            local_network,
        ))
    }

    /// [`Self::of_answer`] for an answer to `apdu`, checked against the
    /// pending transactions in `tsm`. A request routed to this network while
    /// its number was unknown stays keyed to the routed form once the number
    /// is learned. When no transaction with the answer's invoke ID is keyed
    /// to the direct station and one is keyed to that routed form
    /// ([`CanonicalPeer::routed_alias`]), the answer belongs to it, as it did
    /// before the number was known (#1465). The TSM is locked only for an
    /// answer relayed with this network's number as SNET.
    pub(crate) async fn of_answer_in(
        tsm: &SyncMutex<Tsm>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        local_network: Option<u16>,
        apdu: &Apdu,
    ) -> Self {
        let answer = Self::of_answer(source_mac, source_network, local_network);
        let alias = CanonicalPeer::routed_alias(source_network, local_network);
        let (Some(invoke_id), Some(alias)) = (response_invoke_id(apdu), alias) else {
            return answer;
        };
        let alias = Self::of(alias);
        let tsm = tsm.lock();
        if !tsm.has_transaction(&answer.tsm_mac, invoke_id)
            && tsm.has_transaction(&alias.tsm_mac, invoke_id)
        {
            alias
        } else {
            answer
        }
    }
}

impl ConfirmedTarget<'_> {
    /// This target as it is sent once the client knows `local_network`, the
    /// number of its own network (#1358). A routed target on that network is
    /// a local one: the DADR is a MAC on this link, so the request goes there
    /// with no DNET and not through the router (Clause 6.5.1), since a
    /// non-routing peer drops an NPDU whose DNET names a network (Clause
    /// 6.5.2.1). Its answer then comes from that MAC with no SNET, or through
    /// a router with this network as its SNET, and either completes the
    /// transaction keyed to the local peer (#1465). Any other target, and
    /// every target while the number is unknown, is unchanged.
    pub(super) fn localized(self, local_network: Option<u16>) -> Self {
        match self {
            Self::Routed {
                dest_network,
                dest_mac,
                ..
            } if Some(dest_network) == local_network => Self::Local { mac: dest_mac },
            target => target,
        }
    }

    pub(super) fn transaction_peer(self) -> TransactionPeer {
        TransactionPeer::of(match self {
            Self::Local { mac } => CanonicalPeer::direct(mac),
            Self::Routed {
                dest_network,
                dest_mac,
                ..
            } => CanonicalPeer::routed(dest_network, dest_mac),
        })
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

    fn address(network: u16, mac: &[u8]) -> NpduAddress {
        NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }
    }

    #[test]
    fn outbound_and_inbound_transaction_identities_agree() {
        let direct = ConfirmedTarget::Local { mac: &[1, 2, 3] }.transaction_peer();
        let direct_response = TransactionPeer::of_answer(&[1, 2, 3], None, None);
        assert_eq!(direct.tsm_mac, direct_response.tsm_mac);
        assert_eq!(direct.canonical, direct_response.canonical);

        let routed = ConfirmedTarget::Routed {
            router_mac: &[9],
            dest_network: 42,
            dest_mac: &[4, 5],
        }
        .transaction_peer();
        let routed_response = TransactionPeer::of_answer(&[8], Some(&address(42, &[4, 5])), None);
        assert_eq!(routed.tsm_mac, routed_response.tsm_mac);
        assert_eq!(routed.canonical, routed_response.canonical);
    }

    /// A request routed to the known local network goes to the DADR and is
    /// keyed to it, and an answer relayed back with that network as its SNET
    /// carries the same key as one sent straight from the DADR (#1465).
    #[test]
    fn a_relayed_answer_from_this_network_matches_the_localized_request() {
        let request = ConfirmedTarget::Routed {
            router_mac: &[9],
            dest_network: 42,
            dest_mac: &[4, 5],
        }
        .localized(Some(42))
        .transaction_peer();
        for answer in [
            TransactionPeer::of_answer(&[9], Some(&address(42, &[4, 5])), Some(42)),
            TransactionPeer::of_answer(&[4, 5], None, Some(42)),
        ] {
            assert_eq!(answer.tsm_mac, request.tsm_mac);
            assert_eq!(answer.canonical, request.canonical);
        }
        // Another SNET, another SADR, or an unknown number keys elsewhere.
        for other in [
            TransactionPeer::of_answer(&[9], Some(&address(43, &[4, 5])), Some(42)),
            TransactionPeer::of_answer(&[9], Some(&address(42, &[4, 6])), Some(42)),
            TransactionPeer::of_answer(&[9], Some(&address(42, &[4, 5])), None),
        ] {
            assert_ne!(other.tsm_mac, request.tsm_mac);
            assert_ne!(other.canonical, request.canonical);
        }
    }

    #[test]
    fn only_a_routed_target_on_the_known_local_network_is_localized() {
        let routed = |dest_network| ConfirmedTarget::Routed {
            router_mac: &[9],
            dest_network,
            dest_mac: &[4, 5],
        };
        assert!(matches!(
            routed(42).localized(Some(42)),
            ConfirmedTarget::Local { mac: &[4, 5] }
        ));
        for local_network in [Some(7), None] {
            assert!(matches!(
                routed(42).localized(local_network),
                ConfirmedTarget::Routed {
                    router_mac: &[9],
                    dest_network: 42,
                    dest_mac: &[4, 5],
                }
            ));
        }
        assert!(matches!(
            ConfirmedTarget::Local { mac: &[1] }.localized(Some(42)),
            ConfirmedTarget::Local { mac: &[1] }
        ));
    }

    #[test]
    fn empty_routed_source_falls_back_to_immediate_mac() {
        let identity = TransactionPeer::of_answer(&[7, 8], Some(&address(99, &[])), None);
        assert_eq!(identity.tsm_mac, MacAddr::from_slice(&[7, 8]));
        assert_eq!(identity.canonical, CanonicalPeer::direct(&[7, 8]));
    }

    /// A TSM key copied from a peer keeps every address length, inline or
    /// spilled, and owns its bytes: changing one copy leaves the other
    /// alone (#1978).
    #[test]
    fn transaction_keys_preserve_full_addresses_and_independent_owners() {
        let mut keys = std::collections::HashMap::new();
        for len in [0, 1, 3, 6, 7, 18, 23, 64] {
            let bytes: Vec<u8> = (0..len).map(|index| index as u8).collect();
            let mut direct = TransactionPeer::of(CanonicalPeer::direct(&bytes));
            assert_eq!(direct.tsm_mac.as_slice(), bytes);
            assert_eq!(direct.tsm_mac.spilled(), len > 6);
            assert!(keys.insert(direct.tsm_mac.clone(), len).is_none());
            let CanonicalPeer::Direct(address) = &mut direct.canonical else {
                unreachable!();
            };
            address.push(0xff);
            assert_eq!(direct.tsm_mac.as_slice(), bytes);
            assert_eq!(keys.get(&direct.tsm_mac), Some(&len));
            direct.tsm_mac.push(0xfe);
            assert_eq!(address.last(), Some(&0xff));

            for network in [42_u16, 43] {
                let mut routed = TransactionPeer::of(CanonicalPeer::routed(network, &bytes));
                let [high, low] = network.to_be_bytes();
                let mut expected = vec![0xff, b'R', high, low, len as u8];
                expected.extend_from_slice(&bytes);
                assert_eq!(routed.tsm_mac.as_slice(), expected);
                assert!(keys.insert(routed.tsm_mac.clone(), len).is_none());
                let CanonicalPeer::Routed { address, .. } = &mut routed.canonical else {
                    unreachable!();
                };
                address.push(0xff);
                assert_eq!(routed.tsm_mac.as_slice(), expected);
                routed.tsm_mac.push(0xfe);
                assert_eq!(address.last(), Some(&0xff));
            }
        }
        assert_eq!(keys.len(), 24);
    }
}
