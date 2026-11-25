//! The segment timer's expiry, called on its own (#2558). The wait reaches
//! it only when the timer and a segment's progress are ready in the same
//! poll and the select takes the timer, a race no end-to-end test can pin:
//! here the transaction's state is set by hand first.

use super::*;
use bacnet_transport::test_transport::{StartMode, TestTransport};

const PEER: &[u8] = &[0x02];

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
