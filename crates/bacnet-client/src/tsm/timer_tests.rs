//! The TSM's timers and owners against segmented admission and reused
//! invoke IDs, split from `tests.rs` for the file-size cap.

use super::tests::read_ack;
use super::*;

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
        registration.progress.current(),
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
        registration.progress.current(),
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
    assert!(
        tsm.begin_final_segment_send(&mac, invoke_id, &first.owner)
            .is_none()
    );
    assert!(!tsm.finish_segmented_request(&mac, invoke_id, &first.owner));
    assert!(!tsm.reset_segmented_response(&mac, invoke_id, &first.owner));
    assert_eq!(
        tsm.complete_transaction_for_owner(
            &mac,
            invoke_id,
            &first.owner,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            read_ack(),
        ),
        CompletionOutcome::NoTransaction
    );
    assert_eq!(tsm.pending_count(), 1);
    assert_eq!(
        replacement.progress.current(),
        TransactionProgress::SegmentedResponse { generation }
    );
    assert_eq!(
        tsm.complete_transaction_for_owner(
            &mac,
            invoke_id,
            &replacement.owner,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            read_ack(),
        ),
        CompletionOutcome::Delivered
    );
    assert!(matches!(
        replacement.response.await.unwrap(),
        TsmResponse::ComplexAck { .. }
    ));
}
