use super::*;
use tokio::time::{timeout, Duration};

fn assert_initial_response_aborted(
    admission: SegmentedResponseAdmission,
    expected_wire_reason: AbortReason,
) {
    match admission {
        SegmentedResponseAdmission::InitialResponseAborted { wire_reason } => {
            assert_eq!(wire_reason, expected_wire_reason);
        }
        other => panic!("expected aborted initial response, got {other:?}"),
    }
}

#[test]
fn allocate_invoke_id_sequential() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = [127, 0, 0, 1, 0xBA, 0xC0];
    let id1 = tsm.allocate_invoke_id(&mac);
    let id2 = tsm.allocate_invoke_id(&mac);
    assert_eq!(id1, Some(0));
    assert_eq!(id2, Some(1));
}

#[test]
fn allocate_invoke_id_per_destination() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac_a = [10, 0, 0, 1, 0xBA, 0xC0];
    let mac_b = [10, 0, 0, 2, 0xBA, 0xC0];
    let id_a = tsm.allocate_invoke_id(&mac_a);
    let id_b = tsm.allocate_invoke_id(&mac_b);
    assert_eq!(id_a, Some(0));
    assert_eq!(id_b, Some(0));
}

#[test]
fn allocate_invoke_id_wraps() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = [127, 0, 0, 1, 0xBA, 0xC0];
    for i in 0..256 {
        assert_eq!(tsm.allocate_invoke_id(&mac), Some(i as u8));
    }
    assert_eq!(tsm.allocate_invoke_id(&mac), None);
}

#[test]
fn release_makes_id_available() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = [127, 0, 0, 1, 0xBA, 0xC0];
    let id0 = tsm.allocate_invoke_id(&mac).unwrap();
    let id1 = tsm.allocate_invoke_id(&mac).unwrap();
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    tsm.release_invoke_id(&mac, id0);
    let id2 = tsm.allocate_invoke_id(&mac).unwrap();
    assert_eq!(id2, 2);
    tsm.release_invoke_id(&mac, id1);
    tsm.release_invoke_id(&mac, id2);
    let id3 = tsm.allocate_invoke_id(&mac).unwrap();
    assert_eq!(id3, 0);
}

#[tokio::test]
async fn register_and_complete_transaction() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let rx = tsm.register_transaction(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let response = TsmResponse::ComplexAck {
        service_data: Bytes::from_static(&[0xDE, 0xAD]),
    };
    let outcome = tsm.complete_transaction(
        &mac,
        invoke_id,
        Some(ConfirmedServiceChoice::READ_PROPERTY),
        response,
    );
    assert_eq!(outcome, CompletionOutcome::Delivered);
    match rx.await.unwrap() {
        TsmResponse::ComplexAck { service_data } => assert_eq!(service_data, vec![0xDE, 0xAD]),
        _ => panic!("Expected ComplexAck"),
    }
}

#[tokio::test]
async fn mismatched_service_choice_leaves_the_transaction_pending() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let rx = tsm.register_transaction(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let outcome = tsm.complete_transaction(
        &mac,
        invoke_id,
        Some(ConfirmedServiceChoice::WRITE_PROPERTY),
        TsmResponse::ComplexAck {
            service_data: Bytes::from_static(&[0xBA, 0xD0]),
        },
    );
    assert_eq!(
        outcome,
        CompletionOutcome::ServiceChoiceMismatch {
            expected: ConfirmedServiceChoice::READ_PROPERTY,
            observed: ConfirmedServiceChoice::WRITE_PROPERTY,
        }
    );
    assert_eq!(tsm.pending_count(), 1, "transaction must stay pending");
    assert_eq!(
        tsm.allocate_invoke_id(&mac),
        Some(invoke_id.wrapping_add(1)),
        "the invoke ID must not have been handed back for reuse"
    );
    let outcome = tsm.complete_transaction(
        &mac,
        invoke_id,
        Some(ConfirmedServiceChoice::READ_PROPERTY),
        TsmResponse::ComplexAck {
            service_data: Bytes::from_static(&[0xDE, 0xAD]),
        },
    );
    assert_eq!(outcome, CompletionOutcome::Delivered);
    match rx.await.unwrap() {
        TsmResponse::ComplexAck { service_data } => {
            assert_eq!(service_data.as_ref(), &[0xDE, 0xAD]);
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    }
}

#[tokio::test]
async fn a_response_without_a_service_choice_completes_any_transaction() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let rx = tsm.register_transaction(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let outcome =
        tsm.complete_transaction(&mac, invoke_id, None, TsmResponse::Reject { reason: 9 });
    assert_eq!(outcome, CompletionOutcome::Delivered);
    assert!(matches!(
        rx.await.unwrap(),
        TsmResponse::Reject { reason: 9 }
    ));
}

#[tokio::test]
async fn segmented_request_rejects_premature_terminal_response_kinds() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);

    for response_kind in 0..3 {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_segmented_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        assert!(
            matches!(
                tsm.admit_terminal_response(&mac, invoke_id, None),
                TerminalResponseAdmission::PrematureSegmentedRequestAborted
            ),
            "response kind {response_kind} must be phase-gated before service correlation"
        );
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Abort { reason }
                if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
        ));
        assert_eq!(tsm.pending_count(), 0);
        assert_eq!(tsm.allocate_invoke_id(&mac), Some(invoke_id));
    }
}

#[tokio::test]
async fn segmented_complex_ack_zero_is_rejected_until_all_segments_are_sent() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_segmented_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );

    assert!(matches!(
        tsm.admit_segmented_complex_ack(&mac, invoke_id, 0, true),
        SegmentedResponseAdmission::InitialResponseAborted {
            wire_reason: AbortReason::INVALID_APDU_IN_THIS_STATE
        }
    ));
    assert!(matches!(
        registration.response.await.unwrap(),
        TsmResponse::Abort { reason }
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
    ));
    assert_eq!(tsm.pending_count(), 0);
}

#[tokio::test]
async fn awaiting_response_initial_segment_admission_matrix() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    for (sequence_number, accepted, expected_wire_reason) in [
        (1, true, AbortReason::INVALID_APDU_IN_THIS_STATE),
        (0, false, AbortReason::SEGMENTATION_NOT_SUPPORTED),
        (1, false, AbortReason::INVALID_APDU_IN_THIS_STATE),
    ] {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        assert_initial_response_aborted(
            tsm.admit_segmented_complex_ack(&mac, invoke_id, sequence_number, accepted),
            expected_wire_reason,
        );
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Abort { reason }
                if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
        ));
        assert_eq!(tsm.pending_count(), 0);
        assert_eq!(tsm.allocate_invoke_id(&mac), Some(invoke_id));
    }
}

#[test]
fn segmented_response_continuations_do_not_repeat_initial_admission() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );

    assert!(matches!(
        tsm.admit_segmented_complex_ack(&mac, invoke_id, 0, true),
        SegmentedResponseAdmission::Active(ref owner) if owner.same_as(&registration.owner)
    ));
    tsm.begin_segmented_response(&mac, invoke_id, &registration.owner)
        .unwrap();
    for (sequence_number, accepted) in [(1, true), (2, false)] {
        assert!(matches!(
            tsm.admit_segmented_complex_ack_for_owner(
                &mac,
                invoke_id,
                sequence_number,
                accepted,
                &registration.owner,
            ),
            SegmentedResponseAdmission::Active(ref owner)
                if owner.same_as(&registration.owner)
        ));
    }
    assert_eq!(tsm.pending_count(), 1);
}

#[tokio::test]
async fn segmented_request_initial_segment_admission_matrix() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    for (sequence_number, accepted, expected_wire_reason) in [
        (0, true, AbortReason::INVALID_APDU_IN_THIS_STATE),
        (0, false, AbortReason::SEGMENTATION_NOT_SUPPORTED),
        (1, false, AbortReason::INVALID_APDU_IN_THIS_STATE),
    ] {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_segmented_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        assert_initial_response_aborted(
            tsm.admit_segmented_complex_ack(&mac, invoke_id, sequence_number, accepted),
            expected_wire_reason,
        );
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Abort { reason }
                if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
        ));
        assert_eq!(tsm.pending_count(), 0);
    }
}

#[tokio::test]
async fn sent_all_segments_still_applies_initial_segment_admission() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    for (sequence_number, accepted, expected_wire_reason) in [
        (0, false, AbortReason::SEGMENTATION_NOT_SUPPORTED),
        (1, false, AbortReason::INVALID_APDU_IN_THIS_STATE),
    ] {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_segmented_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        let mut token = tsm
            .begin_final_segment_send(&mac, invoke_id, &registration.owner)
            .unwrap();
        assert!(tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut token));

        assert_initial_response_aborted(
            tsm.admit_segmented_complex_ack_for_owner(
                &mac,
                invoke_id,
                sequence_number,
                accepted,
                &registration.owner,
            ),
            expected_wire_reason,
        );
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Abort { reason }
                if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
        ));
        assert_eq!(tsm.pending_count(), 0);
    }

    let mut tsm = Tsm::new(TsmConfig::default());
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_segmented_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let mut token = tsm
        .begin_final_segment_send(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert!(tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut token));
    assert!(matches!(
        tsm.admit_segmented_complex_ack_for_owner(
            &mac,
            invoke_id,
            0,
            true,
            &registration.owner,
        ),
        SegmentedResponseAdmission::Active(ref owner) if owner.same_as(&registration.owner)
    ));
}

#[tokio::test]
async fn invalid_initial_segment_resolves_final_send_polling() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    for (sequence_number, accepted, expected_wire_reason) in [
        (0, false, AbortReason::SEGMENTATION_NOT_SUPPORTED),
        (1, true, AbortReason::INVALID_APDU_IN_THIS_STATE),
    ] {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_segmented_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        let mut token = tsm
            .begin_final_segment_send(&mac, invoke_id, &registration.owner)
            .unwrap();
        let (deferred_owner, issue) = match tsm
            .admit_segmented_complex_ack(&mac, invoke_id, 0, true)
        {
            SegmentedResponseAdmission::FinalSegmentSendPolling { owner, issue } => (owner, issue),
            other => panic!("expected final-send deferral, got {other:?}"),
        };

        assert_initial_response_aborted(
            tsm.admit_segmented_complex_ack_for_owner(
                &mac,
                invoke_id,
                sequence_number,
                accepted,
                &deferred_owner,
            ),
            expected_wire_reason,
        );
        assert!(!tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut token));
        drop(token);
        timeout(Duration::from_millis(100), issue.wait_until_polled())
            .await
            .expect("final-send admission waiter remained blocked");
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Abort { reason }
                if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
        ));
    }
}

#[tokio::test]
async fn reject_and_abort_are_valid_throughout_segmented_request() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);

    for response in [
        TsmResponse::Reject { reason: 3 },
        TsmResponse::Abort { reason: 4 },
    ] {
        let mut tsm = Tsm::new(TsmConfig::default());
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_segmented_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        assert_eq!(
            tsm.complete_transaction(&mac, invoke_id, None, response),
            CompletionOutcome::Delivered
        );
        assert!(matches!(
            registration.response.await.unwrap(),
            TsmResponse::Reject { reason: 3 } | TsmResponse::Abort { reason: 4 }
        ));
        assert_eq!(tsm.pending_count(), 0);
    }
}

#[tokio::test]
async fn sent_all_segments_enables_response_correlation_before_final_ack() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_segmented_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let mut token = tsm
        .begin_final_segment_send(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert!(tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut token));

    assert_eq!(
        tsm.complete_transaction(
            &mac,
            invoke_id,
            Some(ConfirmedServiceChoice::WRITE_PROPERTY),
            TsmResponse::SimpleAck,
        ),
        CompletionOutcome::ServiceChoiceMismatch {
            expected: ConfirmedServiceChoice::READ_PROPERTY,
            observed: ConfirmedServiceChoice::WRITE_PROPERTY,
        }
    );
    assert_eq!(tsm.pending_count(), 1);
    assert_eq!(
        tsm.complete_transaction(
            &mac,
            invoke_id,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::SimpleAck,
        ),
        CompletionOutcome::Delivered
    );
    assert!(matches!(
        registration.response.await.unwrap(),
        TsmResponse::SimpleAck
    ));
}

#[test]
fn final_segment_ack_preserves_owner_in_await_confirmation() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_segmented_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert!(matches!(
        tsm.segment_ack_phase(&mac, invoke_id),
        SegmentAckPhase::SegmentedRequest(ref owner) if owner.same_as(&registration.owner)
    ));
    let mut token = tsm
        .begin_final_segment_send(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert!(tsm.mark_final_segment_issued(&mac, invoke_id, &registration.owner, &mut token));
    assert!(tsm.finish_segmented_request(&mac, invoke_id, &registration.owner));
    assert!(matches!(
        tsm.segment_ack_phase(&mac, invoke_id),
        SegmentAckPhase::Outstanding
    ));
    assert!(tsm.owner_is_current(&mac, invoke_id, &registration.owner));
}

#[tokio::test]
async fn complete_unknown_transaction_reports_no_transaction() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let outcome = tsm.complete_transaction(
        &mac,
        42,
        Some(ConfirmedServiceChoice::READ_PROPERTY),
        TsmResponse::SimpleAck,
    );
    assert_eq!(outcome, CompletionOutcome::NoTransaction);
}

#[test]
fn cancel_transaction() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let _rx = tsm.register_transaction(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert_eq!(tsm.pending_count(), 1);
    assert!(tsm.cancel_transaction(&mac, invoke_id));
    assert_eq!(tsm.pending_count(), 0);
}

#[test]
fn segmented_admission_and_request_timeout_are_serialized() {
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let mut admitted_first = Tsm::new(TsmConfig::default());
    let invoke_id = admitted_first.allocate_invoke_id(&mac).unwrap();
    let registration = admitted_first.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let generation = admitted_first
        .begin_segmented_response(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert_eq!(
        *registration.progress.borrow(),
        TransactionProgress::SegmentedResponse { generation }
    );
    assert_eq!(
        admitted_first.expire_request_timer(&mac, invoke_id, &registration.owner, true),
        RequestTimerExpiration::SegmentedResponse { generation }
    );
    assert_eq!(admitted_first.pending_count(), 1);

    let mut timeout_first = Tsm::new(TsmConfig::default());
    let invoke_id = timeout_first.allocate_invoke_id(&mac).unwrap();
    let registration = timeout_first.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert_eq!(
        timeout_first.expire_request_timer(&mac, invoke_id, &registration.owner, true),
        RequestTimerExpiration::TimedOut
    );
    assert_eq!(
        timeout_first.begin_segmented_response(&mac, invoke_id, &registration.owner),
        None
    );
    assert_eq!(timeout_first.pending_count(), 0);
}

#[test]
fn retry_send_failure_recheck_preserves_later_segmented_admission() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert_eq!(
        tsm.expire_request_timer(&mac, invoke_id, &registration.owner, false),
        RequestTimerExpiration::Retry
    );
    let generation = tsm
        .begin_segmented_response(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert_eq!(
        tsm.expire_request_timer(&mac, invoke_id, &registration.owner, true),
        RequestTimerExpiration::SegmentedResponse { generation }
    );
    assert_eq!(tsm.pending_count(), 1);
}

#[test]
fn stale_segment_timer_generation_cannot_cancel_new_activity() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let registration = tsm.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let stale_generation = tsm
        .begin_segmented_response(&mac, invoke_id, &registration.owner)
        .unwrap();
    let current_generation = tsm
        .record_segmented_response_activity(&mac, invoke_id, &registration.owner)
        .unwrap();
    assert_ne!(stale_generation, current_generation);
    assert_eq!(
        tsm.expire_segment_timer(&mac, invoke_id, &registration.owner, stale_generation),
        SegmentTimerExpiration::Activity {
            generation: current_generation
        }
    );
    assert_eq!(tsm.pending_count(), 1);
    assert_eq!(
        *registration.progress.borrow(),
        TransactionProgress::SegmentedResponse {
            generation: current_generation
        }
    );
    assert_eq!(
        tsm.expire_segment_timer(&mac, invoke_id, &registration.owner, current_generation),
        SegmentTimerExpiration::TimedOut
    );
    assert_eq!(tsm.pending_count(), 0);
    assert_eq!(tsm.allocate_invoke_id(&mac), Some(0));
}

#[tokio::test]
async fn stale_owner_cannot_mutate_reused_transaction_key() {
    let mut tsm = Tsm::new(TsmConfig::default());
    let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
    let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
    let first = tsm.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert!(tsm.cancel_transaction_for_owner(&mac, invoke_id, &first.owner));
    assert_eq!(tsm.allocate_invoke_id(&mac), Some(invoke_id));
    let replacement = tsm.register_transaction_with_progress(
        mac.clone(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert!(matches!(
        tsm.admit_segmented_complex_ack_for_owner(&mac, invoke_id, 1, false, &first.owner,),
        SegmentedResponseAdmission::NoTransaction
    ));
    assert_eq!(tsm.pending_count(), 1);
    let generation = tsm
        .begin_segmented_response(&mac, invoke_id, &replacement.owner)
        .unwrap();
    assert_eq!(
        tsm.expire_request_timer(&mac, invoke_id, &first.owner, true),
        RequestTimerExpiration::NoTransaction
    );
    assert_eq!(
        tsm.expire_segment_timer(&mac, invoke_id, &first.owner, generation),
        SegmentTimerExpiration::NoTransaction
    );
    assert!(!tsm.cancel_transaction_for_owner(&mac, invoke_id, &first.owner));
    assert_eq!(
        tsm.begin_segmented_response(&mac, invoke_id, &first.owner),
        None
    );
    assert_eq!(
        tsm.record_segmented_response_activity(&mac, invoke_id, &first.owner),
        None
    );
    assert!(tsm
        .begin_final_segment_send(&mac, invoke_id, &first.owner)
        .is_none());
    assert!(!tsm.finish_segmented_request(&mac, invoke_id, &first.owner));
    assert!(!tsm.reset_segmented_response(&mac, invoke_id, &first.owner));
    assert_eq!(
        tsm.complete_transaction_for_owner(
            &mac,
            invoke_id,
            &first.owner,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::SimpleAck,
        ),
        CompletionOutcome::NoTransaction
    );
    assert_eq!(tsm.pending_count(), 1);
    assert_eq!(
        *replacement.progress.borrow(),
        TransactionProgress::SegmentedResponse { generation }
    );
    assert_eq!(
        tsm.complete_transaction_for_owner(
            &mac,
            invoke_id,
            &replacement.owner,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::SimpleAck,
        ),
        CompletionOutcome::Delivered
    );
    assert!(matches!(
        replacement.response.await.unwrap(),
        TsmResponse::SimpleAck
    ));
}
