use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use bacnet_encoding::apdu::{AbortPdu, ComplexAck, ErrorPdu, RejectPdu, SegmentAck, SimpleAck};
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bytes::Bytes;

use super::*;

const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
const OTHER_SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;

fn peer(value: u8) -> CanonicalPeer {
    CanonicalPeer::direct(&[value])
}

fn requester(peer: CanonicalPeer, policy: TerminalPolicy) -> LeaseMetadata {
    LeaseMetadata::requester(peer, SERVICE, policy)
}

fn simple_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    })
}

fn complex_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice, segmented: bool) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented,
        more_follows: segmented,
        invoke_id,
        sequence_number: segmented.then_some(0),
        proposed_window_size: segmented.then_some(1),
        service_choice,
        service_ack: Bytes::new(),
    })
}

fn error_pdu(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice,
        error_class: ErrorClass::DEVICE,
        error_code: ErrorCode::OTHER,
        error_data: Bytes::new(),
    })
}

fn reject(invoke_id: u8) -> Apdu {
    Apdu::Reject(RejectPdu {
        invoke_id,
        reject_reason: RejectReason::OTHER,
    })
}

fn abort(invoke_id: u8, sent_by_server: bool) -> Apdu {
    Apdu::Abort(AbortPdu {
        sent_by_server,
        invoke_id,
        abort_reason: AbortReason::OTHER,
    })
}

fn segment_ack(invoke_id: u8, sent_by_server: bool) -> Apdu {
    Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    })
}

fn assert_admitted_kind(outcome: AdmissionOutcome, expected: AdmissionKind) -> Admission {
    let AdmissionOutcome::Admitted(admission) = outcome else {
        panic!("expected admitted outcome, got {outcome:?}");
    };
    assert_eq!(admission.kind(), expected);
    admission
}

#[test]
fn global_pool_exhausts_across_peers_and_roles_without_duplicate_ids() {
    let coordinator = OutboundTransactionCoordinator::new();
    let mut tokens = Vec::new();

    for index in 0..INVOKE_ID_COUNT {
        let metadata = if index % 2 == 0 {
            requester(peer(index as u8), TerminalPolicy::SimpleAck)
        } else {
            LeaseMetadata::server_notification(
                CanonicalPeer::routed(index as u16 + 1, &[index as u8]),
                SERVICE,
            )
        };
        tokens.push(coordinator.reserve(metadata).unwrap());
    }

    let invoke_ids: HashSet<_> = tokens.iter().map(|token| token.invoke_id()).collect();
    assert_eq!(invoke_ids.len(), INVOKE_ID_COUNT);
    assert_eq!(coordinator.active_count(), Ok(INVOKE_ID_COUNT));
    assert_eq!(
        coordinator.reserve(requester(peer(0xff), TerminalPolicy::SimpleAck)),
        Err(ReserveError::Exhausted)
    );
}

#[test]
fn notification_abort_releases_for_reuse_and_stale_cleanup_cannot_release_replacement() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(1);
    let original = coordinator
        .reserve(LeaseMetadata::server_notification(
            expected_peer.clone(),
            SERVICE,
        ))
        .unwrap();
    assert_eq!(
        coordinator.admit(&expected_peer, &abort(original.invoke_id(), true)),
        Ok(AdmissionOutcome::DirectionMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &abort(original.invoke_id(), false))
            .unwrap(),
        AdmissionKind::Terminal,
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_eq!(coordinator.complete(original), Ok(ReleaseOutcome::Released));
    assert_eq!(
        coordinator.complete(original),
        Ok(ReleaseOutcome::AlreadyReleased)
    );
    assert_eq!(coordinator.active_count(), Ok(0));

    let mut active = Vec::new();
    for index in 0..INVOKE_ID_COUNT {
        active.push(
            coordinator
                .reserve(requester(peer(index as u8), TerminalPolicy::SimpleAck))
                .unwrap(),
        );
    }
    let replacement = *active
        .iter()
        .find(|token| token.invoke_id() == original.invoke_id())
        .unwrap();
    assert_ne!(replacement.generation, original.generation);
    assert_eq!(
        coordinator.complete(original),
        Ok(ReleaseOutcome::StaleToken)
    );
    assert_eq!(coordinator.active_count(), Ok(INVOKE_ID_COUNT));
    assert_eq!(
        coordinator.complete(replacement),
        Ok(ReleaseOutcome::Released)
    );
    assert_eq!(coordinator.active_count(), Ok(INVOKE_ID_COUNT - 1));
}

#[test]
fn canonical_peer_uses_routed_source_instead_of_immediate_router() {
    let source = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0xaa, 0xbb]),
    };
    let through_first_router = CanonicalPeer::from_source(&[1], Some(&source));
    let through_second_router = CanonicalPeer::from_source(&[2], Some(&source));

    assert_eq!(through_first_router, through_second_router);
    assert_eq!(
        through_first_router,
        CanonicalPeer::routed(200, &[0xaa, 0xbb])
    );
    assert_ne!(CanonicalPeer::from_source(&[1], None), peer(2));
    assert_eq!(CanonicalPeer::from_source(&[1], None), peer(1));
}

#[test]
fn simple_ack_claims_once_and_completion_is_idempotent() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(1);
    let token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    let response = simple_ack(token.invoke_id(), SERVICE);

    let admission = assert_admitted_kind(
        coordinator.admit(&expected_peer, &response).unwrap(),
        AdmissionKind::Terminal,
    );
    assert_eq!(admission.token(), token);
    assert_eq!(admission.metadata().owner(), LeaseOwner::Requester);
    assert_eq!(
        coordinator.admit(&expected_peer, &response),
        Ok(AdmissionOutcome::DuplicateTerminal)
    );
    assert_eq!(coordinator.complete(token), Ok(ReleaseOutcome::Released));
    assert_eq!(
        coordinator.complete(token),
        Ok(ReleaseOutcome::AlreadyReleased)
    );
    assert_eq!(coordinator.active_count(), Ok(0));
}

#[test]
fn segmented_complex_ack_and_segment_ack_remain_non_terminal() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(2);
    let token = coordinator
        .reserve(LeaseMetadata::segmented_requester(
            expected_peer.clone(),
            SERVICE,
            TerminalPolicy::ComplexAck,
        ))
        .unwrap();
    let segment = complex_ack(token.invoke_id(), SERVICE, true);
    let request_ack = segment_ack(token.invoke_id(), true);

    for pdu in [&segment, &segment, &request_ack, &request_ack] {
        assert_admitted_kind(
            coordinator.admit(&expected_peer, pdu).unwrap(),
            AdmissionKind::NonTerminal,
        );
    }
    assert_eq!(coordinator.active_count(), Ok(1));

    let terminal = complex_ack(token.invoke_id(), SERVICE, false);
    assert_admitted_kind(
        coordinator.admit(&expected_peer, &terminal).unwrap(),
        AdmissionKind::Terminal,
    );
    assert_eq!(
        coordinator.admit(&expected_peer, &terminal),
        Ok(AdmissionOutcome::DuplicateTerminal)
    );
    assert_eq!(coordinator.complete(token), Ok(ReleaseOutcome::Released));
}

#[test]
fn server_notification_accepts_simple_ack_but_never_complex_ack() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(3);
    let token = coordinator
        .reserve(LeaseMetadata::server_notification(
            expected_peer.clone(),
            SERVICE,
        ))
        .unwrap();

    assert_eq!(
        coordinator.admit(
            &expected_peer,
            &complex_ack(token.invoke_id(), SERVICE, false)
        ),
        Ok(AdmissionOutcome::OwnerMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &simple_ack(token.invoke_id(), SERVICE))
            .unwrap(),
        AdmissionKind::Terminal,
    );
    assert_eq!(coordinator.complete(token), Ok(ReleaseOutcome::Released));

    let complex_token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::ComplexAck))
        .unwrap();
    assert_eq!(
        coordinator.admit(
            &expected_peer,
            &simple_ack(complex_token.invoke_id(), SERVICE)
        ),
        Ok(AdmissionOutcome::PolicyMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_admitted_kind(
        coordinator
            .admit(
                &expected_peer,
                &complex_ack(complex_token.invoke_id(), SERVICE, false),
            )
            .unwrap(),
        AdmissionKind::Terminal,
    );
    assert_eq!(
        coordinator.complete(complex_token),
        Ok(ReleaseOutcome::Released)
    );
}

#[test]
fn error_reject_and_requester_abort_are_terminal_with_required_checks() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(4);

    let error_token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::ComplexAck))
        .unwrap();
    assert_eq!(
        coordinator.admit(
            &expected_peer,
            &error_pdu(error_token.invoke_id(), OTHER_SERVICE)
        ),
        Ok(AdmissionOutcome::ServiceMismatch {
            expected: SERVICE,
            observed: OTHER_SERVICE,
        })
    );
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &error_pdu(error_token.invoke_id(), SERVICE))
            .unwrap(),
        AdmissionKind::Terminal,
    );
    coordinator.complete(error_token).unwrap();

    let reject_token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &reject(reject_token.invoke_id()))
            .unwrap(),
        AdmissionKind::Terminal,
    );
    coordinator.complete(reject_token).unwrap();

    let abort_token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    assert_eq!(
        coordinator.admit(&expected_peer, &abort(abort_token.invoke_id(), false)),
        Ok(AdmissionOutcome::DirectionMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &abort(abort_token.invoke_id(), true))
            .unwrap(),
        AdmissionKind::Terminal,
    );
}

#[test]
fn segment_ack_requires_server_direction_and_segmented_request_owner() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(5);
    let token = coordinator
        .reserve(LeaseMetadata::segmented_requester(
            expected_peer.clone(),
            SERVICE,
            TerminalPolicy::SimpleAck,
        ))
        .unwrap();
    assert_eq!(
        coordinator.admit(&expected_peer, &segment_ack(token.invoke_id(), false)),
        Ok(AdmissionOutcome::DirectionMismatch)
    );
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &segment_ack(token.invoke_id(), true))
            .unwrap(),
        AdmissionKind::NonTerminal,
    );

    let unsegmented = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    assert_eq!(
        coordinator.admit(&expected_peer, &segment_ack(unsegmented.invoke_id(), true)),
        Ok(AdmissionOutcome::OwnerMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(2));
}

#[test]
fn unknown_peer_service_and_policy_mismatches_do_not_mutate_leases() {
    let coordinator = OutboundTransactionCoordinator::new();
    let expected_peer = peer(6);
    assert_eq!(
        coordinator.admit(&expected_peer, &simple_ack(99, SERVICE)),
        Ok(AdmissionOutcome::UnknownInvokeId)
    );

    let token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    assert_eq!(
        coordinator.admit(&peer(7), &simple_ack(token.invoke_id(), SERVICE)),
        Ok(AdmissionOutcome::PeerMismatch)
    );
    assert_eq!(
        coordinator.admit(
            &expected_peer,
            &simple_ack(token.invoke_id(), OTHER_SERVICE)
        ),
        Ok(AdmissionOutcome::ServiceMismatch {
            expected: SERVICE,
            observed: OTHER_SERVICE,
        })
    );
    assert_eq!(
        coordinator.admit(
            &expected_peer,
            &complex_ack(token.invoke_id(), SERVICE, false)
        ),
        Ok(AdmissionOutcome::PolicyMismatch)
    );
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_admitted_kind(
        coordinator
            .admit(&expected_peer, &simple_ack(token.invoke_id(), SERVICE))
            .unwrap(),
        AdmissionKind::Terminal,
    );
}

#[test]
fn repeated_reserve_release_and_hostile_mismatch_loops_remain_bounded() {
    let coordinator = OutboundTransactionCoordinator::new();
    for _ in 0..10_000 {
        let token = coordinator
            .reserve(requester(peer(8), TerminalPolicy::SimpleAck))
            .unwrap();
        assert_eq!(coordinator.cancel(token), Ok(ReleaseOutcome::Released));
    }
    assert_eq!(coordinator.active_count(), Ok(0));

    let expected_peer = peer(9);
    let token = coordinator
        .reserve(requester(expected_peer.clone(), TerminalPolicy::SimpleAck))
        .unwrap();
    for _ in 0..10_000 {
        assert_eq!(
            coordinator.admit(
                &expected_peer,
                &simple_ack(token.invoke_id(), OTHER_SERVICE)
            ),
            Ok(AdmissionOutcome::ServiceMismatch {
                expected: SERVICE,
                observed: OTHER_SERVICE,
            })
        );
    }
    assert_eq!(coordinator.active_count(), Ok(1));
    assert_eq!(coordinator.release(token), Ok(ReleaseOutcome::Released));
    assert_eq!(
        coordinator.cancel(token),
        Ok(ReleaseOutcome::AlreadyReleased)
    );
}

#[test]
fn generation_overflow_and_poisoning_fail_closed() {
    let exhausted_generation = OutboundTransactionCoordinator::new();
    exhausted_generation.state.lock().unwrap().last_generation = u64::MAX;
    assert_eq!(
        exhausted_generation.reserve(requester(peer(10), TerminalPolicy::SimpleAck)),
        Err(ReserveError::GenerationExhausted)
    );
    assert_eq!(exhausted_generation.active_count(), Ok(0));

    let poisoned = Arc::new(OutboundTransactionCoordinator::new());
    let panic_owner = Arc::clone(&poisoned);
    assert!(thread::spawn(move || {
        let _guard = panic_owner.state.lock().unwrap();
        panic!("poison coordinator for explicit failure-path coverage");
    })
    .join()
    .is_err());
    assert_eq!(
        poisoned.reserve(requester(peer(11), TerminalPolicy::SimpleAck)),
        Err(ReserveError::StatePoisoned)
    );
    assert_eq!(
        poisoned.active_count(),
        Err(CoordinatorError::StatePoisoned)
    );
}

#[test]
fn arc_contention_preserves_global_uniqueness_and_active_count() {
    const THREADS: usize = 16;
    const LEASES_PER_THREAD: usize = INVOKE_ID_COUNT / THREADS;

    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let mut workers = Vec::new();
    for worker in 0..THREADS {
        let coordinator = Arc::clone(&coordinator);
        workers.push(thread::spawn(move || {
            let mut tokens = Vec::new();
            for lease in 0..LEASES_PER_THREAD {
                let address = [worker as u8, lease as u8];
                tokens.push(
                    coordinator
                        .reserve(requester(
                            CanonicalPeer::direct(&address),
                            TerminalPolicy::SimpleAck,
                        ))
                        .unwrap(),
                );
            }
            tokens
        }));
    }

    let tokens: Vec<_> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();
    let invoke_ids: HashSet<_> = tokens.iter().map(|token| token.invoke_id()).collect();
    assert_eq!(tokens.len(), INVOKE_ID_COUNT);
    assert_eq!(invoke_ids.len(), INVOKE_ID_COUNT);
    assert_eq!(coordinator.active_count(), Ok(INVOKE_ID_COUNT));

    for token in tokens {
        assert_eq!(coordinator.release(token), Ok(ReleaseOutcome::Released));
    }
    assert_eq!(coordinator.active_count(), Ok(0));
}
