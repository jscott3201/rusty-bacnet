//! The segment timer's expiry, called on its own (#2558). The wait reaches
//! it only when the timer and a segment's progress are ready in the same
//! poll and the select takes the timer, a race no end-to-end test can pin:
//! here the transaction's state is set by hand first. And a retry that its
//! stopped link refuses (#1833).

use super::*;
use bacnet_transport::test_transport::{StartMode, TestTransport};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Lifecycle;
use bacnet_types::primitives::ObjectIdentifier;
use std::sync::Arc;
use std::time::Duration;

const PEER: &[u8] = &[0x02];

/// A request that went out, whose link then stopped while the client ran
/// on (#1833): the link refuses its retry for its state, and the request
/// ends cut off as one that went out, which the device may have carried
/// out, not with that refusal, which says nothing was done. A request
/// after the stop gets the refusal itself: nothing of it left.
#[tokio::test(start_paused = true)]
async fn a_retry_its_stopped_link_refuses_ends_the_request_cut_off_as_sent() {
    let transport = TestTransport::builder()
        .local_mac(&[0x01])
        .start(StartMode::Fresh(4))
        .build();
    let sent = transport.handle().sent();
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 1,
        ..ClientConfig::default()
    };
    let client = Arc::new(BACnetClient::start(config, transport).await.unwrap());
    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let present = PropertyIdentifier::PRESENT_VALUE;
    let read = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.read_property(PEER, object, present, None).await }
    });
    while sent.is_empty() {
        tokio::task::yield_now().await;
    }
    client.transport().stop().await.unwrap();
    tokio::time::advance(Duration::from_millis(1_100)).await;
    let ended = read.await.unwrap();
    assert!(
        matches!(ended, Err(Error::RequestCutOff { sent: true })),
        "{ended:?}"
    );
    assert_eq!(sent.len(), 1, "the retry went nowhere");
    let refused = client.read_property(PEER, object, present, None).await;
    assert!(
        matches!(refused, Err(Error::State(Lifecycle::NotRunning, _))),
        "{refused:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_segment_timer_expiry_follows_the_transaction_s_phase_and_generation() {
    let transport = TestTransport::builder()
        .local_mac(&[0x01])
        .start(StartMode::Fresh(4))
        .build();
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    let (invoke_id, registration) = client
        .tsm
        .lock()
        .register_coordinated_transaction_with_progress(
            MacAddr::from_slice(PEER),
            CanonicalPeer::direct(PEER),
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
        )
        .unwrap();
    let mut response_rx = registration.response;
    let owner = registration.owner;
    let tsm_mac = MacAddr::from_slice(PEER);
    let wait_for = ConfirmedWait {
        target: ConfirmedTarget::Local { mac: PEER },
        tsm_mac: &tsm_mac,
        invoke_id,
        owner: &owner,
        retry_apdu: None,
    };

    // The timer ran out for the first segment, and another came since: the
    // wait goes on, timed from the later one.
    let first = client
        .tsm
        .lock()
        .begin_segmented_response(PEER, invoke_id, &owner)
        .unwrap();
    let later = client
        .tsm
        .lock()
        .record_segmented_response_activity(PEER, invoke_id, &owner)
        .unwrap();
    assert_ne!(first, later);
    match client
        .segment_timer_expired(&wait_for, first, &mut response_rx)
        .await
    {
        Expired::Wait(TransactionProgress::SegmentedResponse { generation }) => {
            assert_eq!(generation, later);
        }
        Expired::Wait(TransactionProgress::AwaitingResponse) => {
            panic!("activity since the timer was set went back to AWAIT_CONFIRMATION")
        }
        Expired::End(result) => {
            panic!("activity since the timer was set ended the wait: {result:?}")
        }
    }

    // Back in AWAIT_CONFIRMATION, the wait goes on there.
    assert!(
        client
            .tsm
            .lock()
            .reset_segmented_response(PEER, invoke_id, &owner)
    );
    assert!(matches!(
        client
            .segment_timer_expired(&wait_for, later, &mut response_rx)
            .await,
        Expired::Wait(TransactionProgress::AwaitingResponse)
    ));
    // Neither expiry ended the transaction.
    assert_eq!(client.tsm.lock().pending_count(), 1);
}
