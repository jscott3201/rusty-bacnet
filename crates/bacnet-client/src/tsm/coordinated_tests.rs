use std::collections::HashSet;
use std::sync::Arc;

use bacnet_encoding::apdu::{AbortPdu, Apdu, ComplexAck, SegmentAck, SimpleAck};
use bacnet_endpoint_core::coordinator::{
    CanonicalPeer, OutboundTransactionCoordinator, ReserveError,
};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bytes::Bytes;

use super::coordinated::{CoordinatedRegistrationError, SegmentedAckArrival};
use super::*;

fn coordinated_tsm() -> (Tsm, Arc<OutboundTransactionCoordinator>) {
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    (
        Tsm::new_coordinated(TsmConfig::default(), Arc::clone(&coordinator)),
        coordinator,
    )
}

fn register(
    tsm: &mut Tsm,
    mac: &[u8],
    peer: CanonicalPeer,
    service: ConfirmedServiceChoice,
    segmented: bool,
) -> (u8, TransactionRegistration) {
    tsm.register_coordinated_transaction_with_progress(
        MacAddr::from_slice(mac),
        peer,
        service,
        segmented,
    )
    .unwrap()
}

fn complete_phase_gated(
    tsm: &mut Tsm,
    mac: &[u8],
    peer: &CanonicalPeer,
    apdu: &Apdu,
    response: TsmResponse,
) -> CoordinatedCompletion {
    let invoke_id = match apdu {
        Apdu::SimpleAck(pdu) => pdu.invoke_id,
        Apdu::ComplexAck(pdu) => pdu.invoke_id,
        _ => panic!("test helper requires an acknowledgment"),
    };
    let owner = match tsm.admit_terminal_response(mac, invoke_id, None) {
        TerminalResponseAdmission::Active(owner) => owner,
        other => panic!("unexpected local admission: {other:?}"),
    };
    tsm.complete_coordinated_terminal_response(mac, invoke_id, Some(&owner), peer, apdu, response)
}

#[test]
fn global_capacity_spans_direct_and_routed_peers_and_reuses_exactly() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let mut registrations = Vec::new();
    let mut invoke_ids = HashSet::new();

    for index in 0..256u16 {
        let mac = index.to_be_bytes().to_vec();
        let peer = if index % 2 == 0 {
            CanonicalPeer::direct(&mac)
        } else {
            CanonicalPeer::routed(index, &mac)
        };
        let (invoke_id, registration) = register(
            &mut tsm,
            &mac,
            peer,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        );
        assert!(invoke_ids.insert(invoke_id));
        registrations.push((mac, registration));
    }
    assert_eq!(invoke_ids.len(), 256);
    assert_eq!(coordinator.active_count().unwrap(), 256);

    let exhausted = match tsm.register_coordinated_transaction_with_progress(
        MacAddr::from_slice(&[0xFE]),
        CanonicalPeer::direct(&[0xFE]),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    ) {
        Ok(_) => panic!("the 257th coordinated registration must fail"),
        Err(error) => error,
    };
    assert_eq!(
        exhausted,
        CoordinatedRegistrationError::Reserve(ReserveError::Exhausted)
    );

    let (first_mac, first) = &registrations[0];
    let first_token = tsm.coordinated_token(first_mac, 0).unwrap();
    let stale_owner = first.owner.clone();
    assert!(tsm.cancel_transaction_for_owner(first_mac, 0, &stale_owner));
    assert_eq!(coordinator.active_count().unwrap(), 255);

    let (replacement_id, replacement) = register(
        &mut tsm,
        first_mac,
        CanonicalPeer::direct(first_mac),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );
    assert_eq!(replacement_id, 0);
    let replacement_token = tsm.coordinated_token(first_mac, replacement_id).unwrap();
    assert_ne!(replacement_token, first_token);
    assert!(!tsm.cancel_transaction_for_owner(first_mac, replacement_id, &stale_owner));
    assert_eq!(coordinator.active_count().unwrap(), 256);
    assert!(tsm.owner_is_current(first_mac, replacement_id, &replacement.owner));

    tsm.cancel_all_transactions();
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[test]
fn retries_retain_the_same_token_and_active_count() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let mac = [1, 2, 3];
    let (invoke_id, registration) = register(
        &mut tsm,
        &mac,
        CanonicalPeer::direct(&mac),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );
    let token = tsm.coordinated_token(&mac, invoke_id).unwrap();

    for _ in 0..8 {
        assert_eq!(
            tsm.expire_request_timer(&mac, invoke_id, &registration.owner, false),
            RequestTimerExpiration::Retry
        );
        assert_eq!(tsm.coordinated_token(&mac, invoke_id), Some(token));
        assert_eq!(coordinator.active_count().unwrap(), 1);
    }

    assert_eq!(
        tsm.expire_request_timer(&mac, invoke_id, &registration.owner, true),
        RequestTimerExpiration::TimedOut
    );
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[tokio::test]
async fn either_ack_completes_simple_and_complex_once() {
    for complex in [false, true] {
        let (mut tsm, coordinator) = coordinated_tsm();
        let mac = [4, u8::from(complex)];
        let peer = CanonicalPeer::direct(&mac);
        let (invoke_id, registration) = register(
            &mut tsm,
            &mac,
            peer.clone(),
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        );
        let (apdu, response) = if complex {
            (
                Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    service_ack: Bytes::from_static(b"ok"),
                }),
                TsmResponse::ComplexAck {
                    service_data: Bytes::from_static(b"ok"),
                },
            )
        } else {
            (
                Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                }),
                TsmResponse::SimpleAck,
            )
        };

        assert_eq!(
            complete_phase_gated(&mut tsm, &mac, &peer, &apdu, response),
            CoordinatedCompletion::Completed(CompletionOutcome::Delivered)
        );
        assert_eq!(coordinator.active_count().unwrap(), 0);
        assert!(registration.response.await.is_ok());
        assert_eq!(
            tsm.complete_coordinated_terminal_response(
                &mac,
                invoke_id,
                None,
                &peer,
                &apdu,
                TsmResponse::SimpleAck,
            ),
            CoordinatedCompletion::Rejected
        );
    }
}

#[test]
fn hostile_terminal_mismatches_leave_the_transaction_active() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let mac = [7];
    let peer = CanonicalPeer::direct(&mac);
    let (invoke_id, registration) = register(
        &mut tsm,
        &mac,
        peer.clone(),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );
    let owner = registration.owner.clone();

    let wrong_service = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    });
    assert!(matches!(
        tsm.complete_coordinated_terminal_response(
            &mac,
            invoke_id,
            Some(&owner),
            &peer,
            &wrong_service,
            TsmResponse::SimpleAck,
        ),
        CoordinatedCompletion::ServiceChoiceMismatch { .. }
    ));

    let matching = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
    });
    assert_eq!(
        tsm.complete_coordinated_terminal_response(
            &mac,
            invoke_id,
            Some(&owner),
            &CanonicalPeer::direct(&[8]),
            &matching,
            TsmResponse::SimpleAck,
        ),
        CoordinatedCompletion::Rejected
    );
    let wrong_direction = Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id,
        abort_reason: AbortReason::OTHER,
    });
    assert_eq!(
        tsm.complete_coordinated_terminal_response(
            &mac,
            invoke_id,
            Some(&owner),
            &peer,
            &wrong_direction,
            TsmResponse::remote(RemoteError::Abort {
                reason: AbortReason::OTHER,
            }),
        ),
        CoordinatedCompletion::Rejected
    );
    assert_eq!(
        tsm.complete_coordinated_terminal_response(
            &[9],
            invoke_id.wrapping_add(1),
            None,
            &peer,
            &matching,
            TsmResponse::SimpleAck,
        ),
        CoordinatedCompletion::Rejected
    );
    assert_eq!(tsm.pending_count(), 1);
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert!(tsm.cancel_transaction_for_owner(&mac, invoke_id, &owner));
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[test]
fn segmented_progress_is_nonterminal_until_valid_reassembly() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let mac = [10];
    let peer = CanonicalPeer::direct(&mac);
    let (invoke_id, registration) = register(
        &mut tsm,
        &mac,
        peer.clone(),
        ConfirmedServiceChoice::READ_PROPERTY,
        true,
    );
    let segment_ack = Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    });
    assert!(matches!(
        tsm.coordinated_segment_ack_phase(
            &mac,
            invoke_id,
            &CanonicalPeer::direct(&[11]),
            &segment_ack,
        ),
        SegmentAckPhase::CoordinatorRejected
    ));
    assert!(matches!(
        tsm.segment_ack_phase(&mac, invoke_id),
        SegmentAckPhase::SegmentedRequest(ref owner) if owner.same_as(&registration.owner)
    ));
    assert!(matches!(
        tsm.coordinated_segment_ack_phase(&mac, invoke_id, &peer, &segment_ack),
        SegmentAckPhase::SegmentedRequest(ref owner) if owner.same_as(&registration.owner)
    ));
    assert_eq!(coordinator.active_count().unwrap(), 1);

    let mut final_issue = tsm
        .begin_final_segment_send(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert!(tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut final_issue,));
    assert!(tsm.finish_segmented_request(&mac, invoke_id, &registration.owner));

    let first = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"a"),
    });
    assert!(matches!(
        tsm.coordinated_admit_segmented_complex_ack(
            &mac,
            invoke_id,
            0,
            true,
            &CanonicalPeer::direct(&[11]),
            &first,
        ),
        SegmentedResponseAdmission::CoordinatorRejected
    ));
    assert!(matches!(
        tsm.segment_ack_phase(&mac, invoke_id),
        SegmentAckPhase::Outstanding
    ));
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert!(matches!(
        tsm.coordinated_admit_segmented_complex_ack(
            &mac, invoke_id, 0, true, &peer, &first,
        ),
        SegmentedResponseAdmission::Active(ref owner) if owner.same_as(&registration.owner)
    ));
    assert_eq!(coordinator.active_count().unwrap(), 1);
    tsm.begin_segmented_response(&mac, invoke_id, &registration.owner)
        .unwrap();

    let final_segment = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: false,
        invoke_id,
        sequence_number: Some(1),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"b"),
    });
    assert!(matches!(
        tsm.coordinated_admit_segmented_complex_ack_for_owner(
            SegmentedAckArrival {
                source_mac: &mac,
                invoke_id,
                sequence_number: 1,
                segmented_response_accepted: true,
            },
            &registration.owner,
            &peer,
            &final_segment,
        ),
        SegmentedResponseAdmission::Active(_)
    ));
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert_eq!(
        tsm.complete_admitted_transaction_for_owner(
            &mac,
            invoke_id,
            &registration.owner,
            Some(ConfirmedServiceChoice::WRITE_PROPERTY),
            TsmResponse::ComplexAck {
                service_data: Bytes::from_static(b"wrong"),
            },
        ),
        CompletionOutcome::ServiceChoiceMismatch {
            expected: ConfirmedServiceChoice::READ_PROPERTY,
            observed: ConfirmedServiceChoice::WRITE_PROPERTY,
        }
    );
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert_eq!(
        tsm.complete_admitted_transaction_for_owner(
            &mac,
            invoke_id,
            &registration.owner,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::ComplexAck {
                service_data: Bytes::from_static(b"ab"),
            },
        ),
        CompletionOutcome::Delivered
    );
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[test]
fn drop_releases_every_coordinated_lease() {
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    {
        let mut tsm = Tsm::new_coordinated(TsmConfig::default(), Arc::clone(&coordinator));
        for mac in [[1], [2], [3]] {
            register(
                &mut tsm,
                &mac,
                CanonicalPeer::direct(&mac),
                ConfirmedServiceChoice::READ_PROPERTY,
                false,
            );
        }
        assert_eq!(coordinator.active_count().unwrap(), 3);
    }
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[tokio::test]
async fn error_service_is_checked_in_coordinated_and_pre_admitted_completion() {
    use bacnet_encoding::apdu::ErrorPdu;
    use bacnet_endpoint_core::coordinator::AdmissionOutcome;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    for pre_admitted in [false, true] {
        let (mut tsm, coordinator) = coordinated_tsm();
        let mac = [7];
        let peer = CanonicalPeer::direct(&mac);
        let (invoke_id, mut registration) = register(
            &mut tsm,
            &mac,
            peer.clone(),
            ConfirmedServiceChoice::WRITE_PROPERTY,
            false,
        );
        let error = |service_choice| {
            Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice,
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::WRITE_ACCESS_DENIED,
                error_data: Bytes::new(),
            })
        };
        let response = || {
            TsmResponse::remote(RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::WRITE_ACCESS_DENIED,
            })
        };
        let wrong = error(ConfirmedServiceChoice::READ_PROPERTY);
        let matching = error(ConfirmedServiceChoice::WRITE_PROPERTY);
        if pre_admitted {
            let AdmissionOutcome::Admitted(admission) =
                coordinator.admit(&peer, &matching).unwrap()
            else {
                panic!()
            };
            assert!(matches!(
                tsm.complete_pre_admitted_terminal_response(&mac, &admission, &wrong, response()),
                CoordinatedCompletion::ServiceChoiceMismatch { .. }
            ));
            assert!(registration.response.try_recv().is_err());
            assert_eq!(tsm.pending_count(), 1);
            assert_eq!(
                tsm.complete_pre_admitted_terminal_response(
                    &mac,
                    &admission,
                    &matching,
                    response()
                ),
                CoordinatedCompletion::Completed(CompletionOutcome::Delivered)
            );
        } else {
            assert!(matches!(
                tsm.complete_coordinated_terminal_response(
                    &mac,
                    invoke_id,
                    Some(&registration.owner),
                    &peer,
                    &wrong,
                    response()
                ),
                CoordinatedCompletion::ServiceChoiceMismatch { .. }
            ));
            assert!(registration.response.try_recv().is_err());
            assert_eq!(tsm.pending_count(), 1);
            assert_eq!(
                tsm.complete_coordinated_terminal_response(
                    &mac,
                    invoke_id,
                    Some(&registration.owner),
                    &peer,
                    &matching,
                    response()
                ),
                CoordinatedCompletion::Completed(CompletionOutcome::Delivered)
            );
        }
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Remote {
                error: RemoteError::Error {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::WRITE_ACCESS_DENIED,
                },
                detail: None
            }
        ));
        assert_eq!(coordinator.active_count().unwrap(), 0);
        assert_eq!(tsm.pending_count(), 0);
    }
}

/// Diverted registrations (#2617): the client's registration is refused
/// as diverted, a diverted one goes through the coordinator and is
/// answered as any other, the node's DeviceCommunicationControl refuses
/// both while it restricts initiation and leaves the diversion when it
/// lifts, and once the seat closes both are refused.
#[test]
fn diverted_registrations_take_the_coordinator_only_when_asked() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let peer = CanonicalPeer::direct(&[7]);
    let service = ConfirmedServiceChoice::READ_PROPERTY;
    let plain = |tsm: &mut Tsm| {
        tsm.register_coordinated_transaction_with_progress(
            MacAddr::from_slice(&[7]),
            CanonicalPeer::direct(&[7]),
            service,
            false,
        )
        .err()
    };
    tsm.divert_registrations();
    assert_eq!(
        plain(&mut tsm),
        Some(CoordinatedRegistrationError::Diverted)
    );
    let (invoke_id, _registration) = tsm
        .register_diverted(MacAddr::from_slice(&[7]), peer.clone(), service, false)
        .unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert_eq!(
        tsm.coordinated_token(&[7], invoke_id).unwrap().invoke_id(),
        invoke_id
    );
    // The diversion stays: the next plain registration is refused again.
    assert_eq!(
        plain(&mut tsm),
        Some(CoordinatedRegistrationError::Diverted)
    );
    // DeviceCommunicationControl refuses both while it restricts, names
    // itself, and leaves the diversion in place when it lifts.
    tsm.restrict_initiation(true);
    assert_eq!(
        plain(&mut tsm),
        Some(CoordinatedRegistrationError::InitiationRestricted)
    );
    assert_eq!(
        tsm.register_diverted(MacAddr::from_slice(&[7]), peer.clone(), service, false)
            .err(),
        Some(CoordinatedRegistrationError::InitiationRestricted)
    );
    tsm.restrict_initiation(false);
    assert_eq!(
        plain(&mut tsm),
        Some(CoordinatedRegistrationError::Diverted)
    );
    assert!(!tsm.initiation_restricted());
    tsm.close_coordinated();
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert!(matches!(
        tsm.register_diverted(MacAddr::from_slice(&[7]), peer, service, false),
        Err(CoordinatedRegistrationError::CoordinatorUnavailable)
    ));
}

/// The node's DeviceCommunicationControl (#2617): while it restricts
/// initiation no transaction is leased, with a refusal that names it; a
/// retry falling due ends its transaction, freeing the invoke ID; and once
/// it is lifted the TSM leases again. A closed TSM stays closed.
#[test]
fn a_restricted_node_leases_nothing_and_withdraws_retries() {
    let (mut tsm, coordinator) = coordinated_tsm();
    let peer = CanonicalPeer::direct(&[7]);
    let (waiting, registration) = register(
        &mut tsm,
        &[7],
        peer.clone(),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );
    assert!(!tsm.initiation_restricted());

    tsm.restrict_initiation(true);
    assert!(tsm.initiation_restricted());
    let refused = tsm
        .register_coordinated_transaction_with_progress(
            MacAddr::from_slice(&[7]),
            peer.clone(),
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        )
        .err()
        .expect("refused");
    assert_eq!(refused, CoordinatedRegistrationError::InitiationRestricted);
    assert!(matches!(
        bacnet_types::error::Error::from(refused),
        bacnet_types::error::Error::InitiationDisabled { sent: false }
    ));
    // The one already waiting gets no retry: it ends, and its ID is free.
    assert_eq!(
        tsm.expire_request_timer(&[7], waiting, &registration.owner, false),
        RequestTimerExpiration::Withdrawn
    );
    assert_eq!(tsm.pending_count(), 0);
    assert_eq!(coordinator.active_count().unwrap(), 0);

    tsm.restrict_initiation(false);
    register(
        &mut tsm,
        &[7],
        peer.clone(),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );

    tsm.close_coordinated();
    tsm.restrict_initiation(false);
    let closed = tsm
        .register_coordinated_transaction_with_progress(
            MacAddr::from_slice(&[7]),
            peer,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        )
        .err()
        .expect("closed");
    assert_eq!(closed, CoordinatedRegistrationError::CoordinatorUnavailable);
}

/// A final timeout is a timeout, restricted or not.
#[test]
fn a_final_timeout_is_no_withdrawal() {
    let (mut tsm, _coordinator) = coordinated_tsm();
    let (waiting, registration) = register(
        &mut tsm,
        &[7],
        CanonicalPeer::direct(&[7]),
        ConfirmedServiceChoice::READ_PROPERTY,
        false,
    );
    tsm.restrict_initiation(true);
    assert_eq!(
        tsm.expire_request_timer(&[7], waiting, &registration.owner, true),
        RequestTimerExpiration::TimedOut
    );
}
