//! Which peer a request comes from ([`CanonicalPeer::from_source`]) and an
//! answer belongs to ([`CanonicalPeer::of_answer`]), and how
//! [`OutboundTransactionCoordinator::admit_from_source`] matches leases with
//! them (#1404, #2831).

use bacnet_encoding::apdu::SimpleAck;

use super::*;

const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
const STATION: [u8; 2] = [0xaa, 0xbb];

fn peer(value: u8) -> CanonicalPeer {
    CanonicalPeer::direct(&[value])
}

/// `STATION` on `network`, as a router's SNET/SADR.
fn relayed(network: u16) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&STATION),
    }
}

#[test]
fn canonical_peer_uses_routed_source_instead_of_immediate_router() {
    let through_first_router = CanonicalPeer::from_source(&[1], Some(&relayed(200)), None);
    let through_second_router = CanonicalPeer::from_source(&[2], Some(&relayed(200)), Some(100));

    assert_eq!(through_first_router, through_second_router);
    assert_eq!(through_first_router, CanonicalPeer::routed(200, &STATION));
    assert_ne!(CanonicalPeer::from_source(&[1], None, None), peer(2));
    assert_eq!(CanonicalPeer::from_source(&[1], None, Some(200)), peer(1));
}

/// Once this network's number is known, a request relayed with it as SNET
/// comes from the station at SADR, whichever router relayed it (#1404).
/// Nothing else changes: another SNET stays routed, and so does every SNET
/// while the number is unknown. An answer is never read this way: its SNET
/// stays routed whatever it names (#2831).
#[test]
fn a_request_relayed_with_this_networks_snet_is_from_the_direct_station() {
    let station = CanonicalPeer::direct(&STATION);
    for router in [&[1][..], &[2]] {
        assert_eq!(
            CanonicalPeer::from_source(router, Some(&relayed(200)), Some(200)),
            station
        );
    }
    assert_eq!(
        CanonicalPeer::from_source(&STATION, None, Some(200)),
        station
    );
    assert_eq!(
        CanonicalPeer::from_source(&[1], Some(&relayed(201)), Some(200)),
        CanonicalPeer::routed(201, &STATION)
    );
    assert_eq!(
        CanonicalPeer::from_source(&[1], Some(&relayed(200)), None),
        CanonicalPeer::routed(200, &STATION)
    );
    let no_sadr = NpduAddress {
        network: 200,
        mac_address: MacAddr::new(),
    };
    assert_eq!(
        CanonicalPeer::from_source(&[1], Some(&no_sadr), Some(200)),
        peer(1)
    );

    assert_eq!(
        CanonicalPeer::of_answer(&[1], Some(&relayed(200))),
        CanonicalPeer::routed(200, &STATION)
    );
    assert_eq!(CanonicalPeer::of_answer(&[1], Some(&no_sadr)), peer(1));
    assert_eq!(CanonicalPeer::of_answer(&STATION, None), station);
}

/// Reserve one requester lease for `peer` and return its invoke ID.
fn lease(coordinator: &OutboundTransactionCoordinator, peer: CanonicalPeer) -> u8 {
    let metadata = LeaseMetadata::requester(peer, SERVICE, TerminalPolicy::SimpleAck);
    coordinator.reserve(metadata).unwrap().invoke_id()
}

fn ack(invoke_id: u8) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: SERVICE,
    })
}

/// An answer relayed with this network's SNET matches only a lease for the
/// routed form a request took while the number was unknown, which is what
/// that answer matched before the number was learned. It never matches a
/// lease for the direct station it names, by admission or by holder, with
/// the number known or not (#2831): only that station's own MAC can. The
/// routed form doesn't widen either: another SNET or station, or another
/// invoke ID, matches nothing.
#[test]
fn a_relayed_answer_matches_only_the_routed_form_it_was_sent_to() {
    let source = relayed(200);
    let direct = OutboundTransactionCoordinator::new();
    let invoke_id = lease(&direct, CanonicalPeer::direct(&STATION));
    for local in [Some(200), None] {
        let outcome = direct
            .admit_from_source(&[9], Some(&source), local, &ack(invoke_id))
            .unwrap();
        assert_eq!(outcome, AdmissionOutcome::PeerMismatch, "{local:?}");
        assert_eq!(
            direct.holder_of_source(&[9], Some(&source), local, invoke_id),
            None
        );
    }
    let AdmissionOutcome::Admitted(_) = direct
        .admit_from_source(&STATION, None, Some(200), &ack(invoke_id))
        .unwrap()
    else {
        panic!("the station's own answer is admitted");
    };

    let keyed = CanonicalPeer::routed(200, &STATION);
    let coordinator = OutboundTransactionCoordinator::new();
    let invoke_id = lease(&coordinator, keyed.clone());
    let miss = |routed: Option<&NpduAddress>, invoke_id: u8| {
        let outcome = coordinator
            .admit_from_source(&[9], routed, Some(200), &ack(invoke_id))
            .unwrap();
        assert!(
            !matches!(outcome, AdmissionOutcome::Admitted(_)),
            "{outcome:?}"
        );
    };
    let other_station = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0xaa, 0xbc]),
    };
    miss(Some(&relayed(201)), invoke_id);
    miss(Some(&other_station), invoke_id);
    miss(Some(&source), invoke_id.wrapping_add(1));
    assert_eq!(
        coordinator.holder_of_source(&[9], Some(&source), Some(200), invoke_id),
        Some(LeaseHolder::Client)
    );
    let AdmissionOutcome::Admitted(admission) = coordinator
        .admit_from_source(&[9], Some(&source), Some(200), &ack(invoke_id))
        .unwrap()
    else {
        panic!("the relayed answer is admitted");
    };
    assert_eq!(admission.metadata().peer(), &keyed, "the peer that matched");
}

/// A direct answer from the station matches a lease keyed to the routed form
/// a request took to a network that has since become this one (#2371), and
/// nothing wider: another station, another or an unknown number, or another
/// invoke ID.
#[test]
fn a_direct_answer_matches_the_routed_form_naming_this_network() {
    assert_eq!(
        CanonicalPeer::direct_alias(&STATION, None, Some(200)),
        Some(CanonicalPeer::routed(200, &STATION))
    );
    assert_eq!(CanonicalPeer::direct_alias(&STATION, None, None), None);
    assert_eq!(
        CanonicalPeer::direct_alias(&STATION, Some(&relayed(200)), Some(200)),
        None
    );
    let coordinator = OutboundTransactionCoordinator::new();
    let invoke_id = lease(&coordinator, CanonicalPeer::routed(200, &STATION));
    for (mac, local, invoke) in [
        (&[0xaa, 0xbc][..], Some(200), invoke_id),
        (&STATION[..], Some(201), invoke_id),
        (&STATION[..], None, invoke_id),
        (&STATION[..], Some(200), invoke_id.wrapping_add(1)),
    ] {
        let outcome = coordinator
            .admit_from_source(mac, None, local, &ack(invoke))
            .unwrap();
        assert!(
            !matches!(outcome, AdmissionOutcome::Admitted(_)),
            "{outcome:?}"
        );
    }
    let AdmissionOutcome::Admitted(admission) = coordinator
        .admit_from_source(&STATION, None, Some(200), &ack(invoke_id))
        .unwrap()
    else {
        panic!("the direct answer is admitted");
    };
    assert_eq!(
        admission.metadata().peer(),
        &CanonicalPeer::routed(200, &STATION)
    );
}

/// A copied peer keeps the variant, the network and every address length,
/// inline or spilled, and owns its bytes (#1978: `Clone` is by hand).
#[test]
fn copied_peers_preserve_arbitrary_address_identity_and_ownership() {
    let mut identities = std::collections::HashMap::new();
    for len in [0, 1, 3, 6, 7, 18, 23, 64] {
        let bytes: Vec<u8> = (0..len).map(|index| index as u8).collect();
        for mut original in [
            CanonicalPeer::direct(&bytes),
            CanonicalPeer::routed(42, &bytes),
            CanonicalPeer::routed(43, &bytes),
        ] {
            let copied = original.clone();
            assert_eq!(copied, original);
            assert!(identities.insert(copied.clone(), len).is_none());
            let (original_mac, copied_mac) = match (&mut original, &copied) {
                (CanonicalPeer::Direct(a), CanonicalPeer::Direct(b)) => (a, b),
                (
                    CanonicalPeer::Routed { address: a, .. },
                    CanonicalPeer::Routed { address: b, .. },
                ) => (a, b),
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
