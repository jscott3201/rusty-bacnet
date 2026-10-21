//! Edges of the per-transaction progress cell (#1727).

use super::super::{Tsm, TsmConfig, TsmResponse};
use super::TransactionProgress;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::MacAddr;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Waker};
use tokio::time::{timeout, Duration};

fn polls_ready<F: Future>(future: F) -> bool {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    future.as_mut().poll(&mut cx).is_ready()
}

fn mac() -> MacAddr {
    MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0])
}

fn segmented_tsm() -> (Tsm, u8, super::super::TransactionRegistration) {
    let mut tsm = Tsm::new(TsmConfig::default());
    let invoke_id = tsm.allocate_invoke_id(&mac()).unwrap();
    let registration = tsm.register_transaction_with_progress(
        mac(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    (tsm, invoke_id, registration)
}

#[test]
fn nothing_published_means_no_change() {
    let (_tsm, _, mut registration) = segmented_tsm();
    assert_eq!(
        registration.progress.borrow_and_update(),
        TransactionProgress::AwaitingResponse
    );
    assert!(!polls_ready(registration.progress.changed()));
}

#[test]
fn publish_before_the_wait_is_seen() {
    let (mut tsm, invoke_id, mut registration) = segmented_tsm();
    registration.progress.borrow_and_update();
    let generation = tsm
        .begin_segmented_response(&mac(), invoke_id, &registration.owner)
        .unwrap();
    assert!(polls_ready(registration.progress.changed()));
    assert_eq!(
        registration.progress.borrow_and_update(),
        TransactionProgress::SegmentedResponse { generation }
    );
    // Seen now; the permit the publish left behind is not a second change.
    assert!(!polls_ready(registration.progress.changed()));
}

#[test]
fn publishing_the_same_value_still_counts_as_a_change() {
    // A segmented request that finishes publishes AWAIT_CONFIRMATION while
    // the progress already reads that, as watch::send_replace did.
    let mut tsm = Tsm::new(TsmConfig::default());
    let invoke_id = tsm.allocate_invoke_id(&mac()).unwrap();
    let mut registration = tsm.register_segmented_transaction_with_progress(
        mac(),
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    registration.progress.borrow_and_update();
    let mut token = tsm
        .begin_final_segment_send(&mac(), invoke_id, &registration.owner)
        .unwrap();
    assert!(tsm.mark_final_segment_issued(&mac(), invoke_id, &registration.owner, &mut token));
    assert!(tsm.finish_segmented_request(&mac(), invoke_id, &registration.owner));
    assert!(polls_ready(registration.progress.changed()));
    assert_eq!(
        registration.progress.borrow_and_update(),
        TransactionProgress::AwaitingResponse
    );
}

#[tokio::test(start_paused = true)]
async fn publish_wakes_a_parked_waiter() {
    let (mut tsm, invoke_id, mut registration) = segmented_tsm();
    registration.progress.borrow_and_update();
    let owner = registration.owner.clone();
    let mut progress = registration.progress;
    let waiter = tokio::spawn(async move {
        progress.changed().await;
        progress.borrow_and_update()
    });
    // Let the waiter park on the Notify before the publish.
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());
    let generation = tsm
        .begin_segmented_response(&mac(), invoke_id, &owner)
        .unwrap();
    let seen = timeout(Duration::from_secs(1), waiter)
        .await
        .expect("publish did not wake the parked waiter")
        .unwrap();
    assert_eq!(seen, TransactionProgress::SegmentedResponse { generation });
}

#[test]
fn progress_after_completion_is_not_published() {
    let (mut tsm, invoke_id, mut registration) = segmented_tsm();
    registration.progress.borrow_and_update();
    tsm.begin_segmented_response(&mac(), invoke_id, &registration.owner)
        .unwrap();
    // The response lands before the waiter looked at that progress.
    tsm.complete_transaction(
        &mac(),
        invoke_id,
        Some(ConfirmedServiceChoice::READ_PROPERTY),
        TsmResponse::SimpleAck,
    );
    // Segment activity that trails the completion finds no transaction.
    assert_eq!(
        tsm.record_segmented_response_activity(&mac(), invoke_id, &registration.owner),
        None
    );
    assert!(!tsm.reset_segmented_response(&mac(), invoke_id, &registration.owner));
    // The waiter still sees the one publish made while pending, then
    // nothing more, and the response is already there to take.
    assert!(polls_ready(registration.progress.changed()));
    registration.progress.borrow_and_update();
    assert!(!polls_ready(registration.progress.changed()));
    assert!(matches!(
        registration.response.try_recv(),
        Ok(TsmResponse::SimpleAck)
    ));
}
