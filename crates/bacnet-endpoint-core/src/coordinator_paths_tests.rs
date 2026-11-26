//! A routed answer counts only through the station its lease's attempt went
//! through ([`OutboundTransactionCoordinator::route_through`], #2831).

use bacnet_encoding::apdu::SimpleAck;

use super::*;

const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;
const ROUTER: [u8; 1] = [0x10];
const STRANGER: [u8; 1] = [0x66];
const DEVICE: [u8; 2] = [0xaa, 0xbb];

fn relayed() -> NpduAddress {
    NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&DEVICE),
    }
}

fn ack(token: LeaseToken) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id: token.invoke_id(),
        service_choice: SERVICE,
    })
}

fn routed_lease(coordinator: &OutboundTransactionCoordinator) -> LeaseToken {
    coordinator
        .reserve(LeaseMetadata::notification(
            CanonicalPeer::routed(300, &DEVICE),
            SERVICE,
        ))
        .unwrap()
}

fn answer_from(
    coordinator: &OutboundTransactionCoordinator,
    station: &[u8],
    token: LeaseToken,
) -> AdmissionOutcome {
    coordinator
        .admit_from_source(station, Some(&relayed()), None, &ack(token))
        .unwrap()
}

#[test]
fn a_routed_answer_through_another_station_is_refused_counted_and_leaves_the_lease_open() {
    let coordinator = OutboundTransactionCoordinator::new();
    let token = routed_lease(&coordinator);
    coordinator.route_through(token, Some(&ROUTER)).unwrap();

    assert_eq!(
        answer_from(&coordinator, &STRANGER, token),
        AdmissionOutcome::PeerMismatch
    );
    assert_eq!(coordinator.off_path_answers().unwrap(), 1);
    assert!(matches!(
        answer_from(&coordinator, &ROUTER, token),
        AdmissionOutcome::Admitted(_)
    ));
    assert_eq!(coordinator.off_path_answers().unwrap(), 1);
}

#[test]
fn a_broadcast_attempt_or_none_recorded_takes_any_station() {
    let coordinator = OutboundTransactionCoordinator::new();
    let unrecorded = routed_lease(&coordinator);
    assert!(matches!(
        answer_from(&coordinator, &STRANGER, unrecorded),
        AdmissionOutcome::Admitted(_)
    ));
    let broadcast = routed_lease(&coordinator);
    coordinator.route_through(broadcast, None).unwrap();
    assert!(matches!(
        answer_from(&coordinator, &STRANGER, broadcast),
        AdmissionOutcome::Admitted(_)
    ));
    assert_eq!(coordinator.off_path_answers().unwrap(), 0);
}

/// Each attempt replaces the one before: a retry sent by broadcast takes any
/// station, and one sent back through a router takes only it.
#[test]
fn the_latest_attempt_decides() {
    let coordinator = OutboundTransactionCoordinator::new();
    let token = routed_lease(&coordinator);
    coordinator.route_through(token, Some(&ROUTER)).unwrap();
    coordinator.route_through(token, None).unwrap();
    coordinator.route_through(token, Some(&STRANGER)).unwrap();
    assert_eq!(
        answer_from(&coordinator, &ROUTER, token),
        AdmissionOutcome::PeerMismatch
    );
    coordinator.route_through(token, None).unwrap();
    assert!(matches!(
        answer_from(&coordinator, &ROUTER, token),
        AdmissionOutcome::Admitted(_)
    ));
}

/// A record belongs to its lease: once released, nothing is recorded for the
/// old token, and the invoke ID's next lease starts unchecked.
#[test]
fn a_record_ends_with_its_lease() {
    let coordinator = OutboundTransactionCoordinator::new();
    let old = routed_lease(&coordinator);
    coordinator.route_through(old, Some(&ROUTER)).unwrap();
    coordinator.release(old).unwrap();
    coordinator.route_through(old, Some(&ROUTER)).unwrap();
    let new = loop {
        let token = routed_lease(&coordinator);
        if token.invoke_id() == old.invoke_id() {
            break token;
        }
    };
    assert!(matches!(
        answer_from(&coordinator, &STRANGER, new),
        AdmissionOutcome::Admitted(_)
    ));
}

/// A direct answer, and a lease to a direct peer, take no path check: the
/// direct peer's own link address is its key.
#[test]
fn a_direct_answer_is_matched_by_its_own_address() {
    let coordinator = OutboundTransactionCoordinator::new();
    let token = coordinator
        .reserve(LeaseMetadata::notification(
            CanonicalPeer::direct(&DEVICE),
            SERVICE,
        ))
        .unwrap();
    coordinator.route_through(token, Some(&ROUTER)).unwrap();
    assert!(matches!(
        coordinator
            .admit_from_source(&DEVICE, None, None, &ack(token))
            .unwrap(),
        AdmissionOutcome::Admitted(_)
    ));
}
