use bacnet_encoding::apdu::{self, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::AbortReason;
use bacnet_types::error::Error;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use super::segmented_receive_lifecycle_tests::{
    expect_segment_ack, response_segment, send_to_client, start_reassembly, CLIENT_MAC, SERVER_MAC,
};
use super::{BACnetClient, ClientConfig};

fn config(segmented_response_accepted: bool) -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 50,
        apdu_retries: 2,
        segmented_response_accepted,
        ..ClientConfig::default()
    }
}

async fn expect_abort(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    invoke_id: u8,
    expected_reason: AbortReason,
) {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for the client Abort")
        .expect("peer channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    match apdu::decode_apdu(npdu.payload).unwrap() {
        Apdu::Abort(abort) => {
            assert!(!abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, expected_reason);
        }
        other => panic!("expected client Abort, got {other:?}"),
    }
}

async fn expect_silence(rx: &mut mpsc::Receiver<ReceivedNpdu>) {
    match timeout(Duration::from_millis(200), rx.recv()).await {
        Err(_) => {}
        Ok(None) => panic!("peer channel closed while checking for retry traffic"),
        Ok(Some(received)) => {
            let npdu = decode_npdu(received.npdu).unwrap();
            let apdu = apdu::decode_apdu(npdu.payload).unwrap();
            panic!("expected no SegmentACK or retry, got {apdu:?}");
        }
    }
}

async fn assert_active_initial_abort(
    sequence_number: u8,
    segmented_response_accepted: bool,
    expected_wire_reason: AbortReason,
) {
    let (request, mut server, mut rx, invoke_id) =
        start_reassembly(config(segmented_response_accepted)).await;

    send_to_client(
        &server,
        &response_segment(invoke_id, sequence_number, true, &[0xAA]),
    )
    .await;
    expect_abort(&mut rx, invoke_id, expected_wire_reason).await;

    let (mut client, result) = timeout(Duration::from_secs(2), request)
        .await
        .expect("request remained pending after initial-response rejection")
        .unwrap();
    assert!(matches!(
        result,
        Err(Error::Abort { reason })
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
    ));
    {
        let mut tsm = client.tsm.lock().await;
        assert_eq!(tsm.pending_count(), 0);
        assert_eq!(tsm.allocate_invoke_id(SERVER_MAC), Some(invoke_id));
        tsm.release_invoke_id(SERVER_MAC, invoke_id);
    }
    expect_silence(&mut rx).await;
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn nonzero_initial_sequence_aborts_an_active_request() {
    assert_active_initial_abort(1, true, AbortReason::INVALID_APDU_IN_THIS_STATE).await;
}

#[tokio::test]
async fn unsupported_initial_segment_completes_with_local_invalid_state_abort() {
    assert_active_initial_abort(0, false, AbortReason::SEGMENTATION_NOT_SUPPORTED).await;
}

#[tokio::test]
async fn nonzero_sequence_precedes_unsupported_capability() {
    assert_active_initial_abort(1, false, AbortReason::INVALID_APDU_IN_THIS_STATE).await;
}

#[tokio::test]
async fn idle_precedes_unsupported_capability() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let mut client = BACnetClient::start(config(false), client_transport)
        .await
        .unwrap();

    send_to_client(&server, &response_segment(42, 0, true, &[0xAA])).await;
    expect_abort(&mut rx, 42, AbortReason::INVALID_APDU_IN_THIS_STATE).await;
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    expect_silence(&mut rx).await;

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn supported_sequence_zero_enters_segmented_response_and_completes() {
    let (request, mut server, mut rx, invoke_id) = start_reassembly(config(true)).await;

    send_to_client(&server, &response_segment(invoke_id, 0, true, b"first-")).await;
    expect_segment_ack(&mut rx, invoke_id, 0).await;
    send_to_client(&server, &response_segment(invoke_id, 1, false, b"response")).await;
    expect_segment_ack(&mut rx, invoke_id, 1).await;

    let (mut client, result) = timeout(Duration::from_secs(2), request)
        .await
        .expect("valid segmented response did not complete")
        .unwrap();
    assert_eq!(result.unwrap().as_ref(), b"first-response");
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
