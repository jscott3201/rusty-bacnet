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

/// An Error, a Reject or an Abort through another station is refused like
/// an acknowledgment: it ends nothing, and the router's own still does.
#[test]
fn an_off_path_error_reject_or_abort_ends_nothing() {
    use bacnet_encoding::apdu::{AbortPdu, ErrorPdu, RejectPdu};
    use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
    let refusals: [fn(u8) -> Apdu; 3] = [
        |invoke_id| {
            Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice: SERVICE,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::OTHER,
                error_data: bytes::Bytes::new(),
            })
        },
        |invoke_id| {
            Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason: RejectReason::OTHER,
            })
        },
        |invoke_id| {
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::OTHER,
            })
        },
    ];
    for refusal in refusals {
        let coordinator = OutboundTransactionCoordinator::new();
        let token = routed_lease(&coordinator);
        coordinator.route_through(token, Some(&ROUTER)).unwrap();
        let apdu = refusal(token.invoke_id());
        let from = |station: &[u8]| {
            coordinator
                .admit_from_source(station, Some(&relayed()), None, &apdu)
                .unwrap()
        };
        assert_eq!(from(&STRANGER), AdmissionOutcome::PeerMismatch, "{apdu:?}");
        assert_eq!(coordinator.active_count().unwrap(), 1);
        assert!(matches!(from(&ROUTER), AdmissionOutcome::Admitted(_)));
        assert_eq!(coordinator.off_path_answers().unwrap(), 1);
    }
}

/// A record is kept only while its lease lasts: the next record prunes the
/// ended ones.
#[test]
fn records_of_ended_leases_are_pruned() {
    let coordinator = OutboundTransactionCoordinator::new();
    let ended: Vec<_> = (0..8).map(|_| routed_lease(&coordinator)).collect();
    for token in &ended {
        coordinator.route_through(*token, Some(&ROUTER)).unwrap();
    }
    assert_eq!(coordinator.recorded_hops(), 8);
    for token in ended {
        coordinator.release(token).unwrap();
    }
    let live = routed_lease(&coordinator);
    coordinator.route_through(live, None).unwrap();
    assert_eq!(coordinator.recorded_hops(), 1);
}

/// The observer's path check takes what admission would: any station for
/// a lease that recorded nothing or went by broadcast, only the router for
/// one that went through it.
#[test]
fn on_path_follows_the_recorded_hop() {
    let coordinator = OutboundTransactionCoordinator::new();
    let unrecorded = routed_lease(&coordinator);
    let through = routed_lease(&coordinator);
    let broadcast = routed_lease(&coordinator);
    coordinator.route_through(through, Some(&ROUTER)).unwrap();
    coordinator.route_through(broadcast, None).unwrap();
    for token in [unrecorded, broadcast] {
        assert!(coordinator.on_path(token.invoke_id(), &STRANGER));
    }
    assert!(coordinator.on_path(through.invoke_id(), &ROUTER));
    assert!(!coordinator.on_path(through.invoke_id(), &STRANGER));
    // An ended lease's record no longer applies.
    coordinator.release(through).unwrap();
    assert!(coordinator.on_path(through.invoke_id(), &STRANGER));
}
