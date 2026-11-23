use std::sync::Arc;

use bacnet_encoding::apdu::Apdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use tokio::time::{Duration, timeout};

use super::segmented_receive_lifecycle_tests::{
    CLIENT_MAC, SERVER_MAC, expect_client_abort, expect_segment_ack, recv_apdu, response_segment,
    send_to_client, start_reassembly,
};
use super::{BACnetClient, ClientConfig};

#[tokio::test]
async fn segment_timer_timeout_ends_the_reassembly_session() {
    let config = ClientConfig {
        apdu_timeout_ms: 100,
        apdu_retries: 3,
        ..ClientConfig::default()
    };
    let (task, server, mut rx, invoke_id) = start_reassembly(config).await;

    send_to_client(&server, &response_segment(invoke_id, 0, true, &[0x01; 8])).await;
    expect_segment_ack(&mut rx, invoke_id, 0).await;

    let (mut client, result) = timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 0);
    assert!(
        timeout(
            Duration::from_secs(2),
            client.segmented_cleanup.wait_processed()
        )
        .await
        .expect("dispatch did not process timeout cleanup"),
        "timeout cleanup did not reclaim the matching reassembly state"
    );
    assert!(
        timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
        "receive SegmentTimer expiry must not send a peer Abort"
    );

    send_to_client(&server, &response_segment(invoke_id, 1, true, &[0x02; 8])).await;
    expect_client_abort(&mut rx, invoke_id, "a segment after SegmentTimer expired").await;
    client.stop().await.unwrap();
}

/// Each response segment restarts the waiter's SegmentTimer from the moment it
/// arrives. A missed progress wakeup would leave the running timer to expire
/// first and only then restart, ending the request a timer period later.
#[tokio::test(start_paused = true)]
async fn response_segment_restarts_segment_timer_from_its_arrival() {
    let config = ClientConfig {
        apdu_timeout_ms: 100,
        apdu_retries: 3,
        ..ClientConfig::default()
    };
    // The waiter allows four segment timeouts in SEGMENTED_CONF.
    let segment_timer = Duration::from_millis(400);
    let (task, server, mut rx, invoke_id) = start_reassembly(config).await;

    send_to_client(&server, &response_segment(invoke_id, 0, true, &[0x01; 8])).await;
    expect_segment_ack(&mut rx, invoke_id, 0).await;
    tokio::time::sleep(segment_timer * 5 / 8).await;
    send_to_client(&server, &response_segment(invoke_id, 1, true, &[0x02; 8])).await;
    expect_segment_ack(&mut rx, invoke_id, 1).await;
    let restarted = tokio::time::Instant::now();

    let (mut client, result) = task.await.unwrap();
    let waited = restarted.elapsed();
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert!(
        waited >= segment_timer && waited < segment_timer + segment_timer / 8,
        "SegmentTimer ran {waited:?} after the last segment, expected {segment_timer:?}"
    );
    client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_request_without_segment_ack_returns_local_tsm_timeout() {
    let retries = 2;
    let config = ClientConfig {
        apdu_timeout_ms: 40,
        apdu_retries: retries,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    };
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(BACnetClient::start(config, client_transport).await.unwrap());

    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });

    let mut invoke_id = None;
    for _ in 0..=retries {
        match recv_apdu(&mut rx, "an unacknowledged outgoing request segment").await {
            Apdu::ConfirmedRequest(segment) => {
                assert!(segment.segmented);
                assert!(segment.more_follows);
                assert_eq!(segment.sequence_number, Some(0));
                if let Some(expected_invoke_id) = invoke_id {
                    assert_eq!(segment.invoke_id, expected_invoke_id);
                }
                invoke_id = Some(segment.invoke_id);
            }
            other => panic!("expected ConfirmedRequest segment, got {other:?}"),
        }
    }
    let invoke_id = invoke_id.expect("at least one request segment");

    let result = timeout(Duration::from_secs(2), request)
        .await
        .expect("SegmentTimer retry exhaustion did not complete the request")
        .unwrap();
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 0);
    assert!(
        !client
            .seg_ack_senders
            .lock()
            .contains_key(&(bacnet_types::MacAddr::from_slice(SERVER_MAC), invoke_id)),
        "SegmentTimer expiry left the SegmentAck sender registered"
    );
    assert!(
        timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
        "SegmentTimer expiry sent an unexpected peer PDU"
    );

    let mut client =
        Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request task retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn delayed_timeout_cleanup_preserves_newer_segmented_response() {
    let config = ClientConfig {
        apdu_timeout_ms: 100,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(BACnetClient::start(config, client_transport).await.unwrap());
    client.segmented_cleanup.delay_next();

    let first_client = Arc::clone(&client);
    let first = tokio::spawn(async move {
        first_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await
    });
    let first_invoke_id = match recv_apdu(&mut rx, "the first request").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    send_to_client(&server, &response_segment(first_invoke_id, 0, true, b"old")).await;
    expect_segment_ack(&mut rx, first_invoke_id, 0).await;
    timeout(
        Duration::from_secs(2),
        client.segmented_cleanup.wait_until_reached(),
    )
    .await
    .expect("first SegmentTimer did not reach delayed cleanup");

    let second_client = Arc::clone(&client);
    let second = tokio::spawn(async move {
        second_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x02])
            .await
    });
    let second_invoke_id = match recv_apdu(&mut rx, "the replacement request").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    assert_ne!(second_invoke_id, first_invoke_id);
    send_to_client(
        &server,
        &response_segment(second_invoke_id, 0, true, b"new-"),
    )
    .await;
    expect_segment_ack(&mut rx, second_invoke_id, 0).await;

    client.segmented_cleanup.release();
    assert!(matches!(
        timeout(Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert!(
        timeout(
            Duration::from_secs(2),
            client.segmented_cleanup.wait_processed()
        )
        .await
        .expect("dispatch did not process delayed cleanup"),
        "owner A cleanup did not reclaim its stale reassembly state"
    );
    assert_eq!(client.tsm.lock().pending_count(), 1);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 1);

    send_to_client(
        &server,
        &response_segment(second_invoke_id, 1, false, b"response"),
    )
    .await;
    expect_segment_ack(&mut rx, second_invoke_id, 1).await;
    let result = timeout(Duration::from_secs(2), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.as_ref(), b"new-response");
    assert!(
        timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
        "old timeout cleanup sent an unexpected peer PDU"
    );

    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("request tasks retained the client"),
    };
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

// The test thread holds the TSM lock across the cancellation, so the
// dispatch task, which takes it to clean up, must run on a worker of its own:
// the lock blocks, and on one thread it would wait for the test forever.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn caller_cancellation_reclaims_reassembly_after_tsm_lock_contention() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap(),
    );

    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await
    });
    let invoke_id = match recv_apdu(&mut rx, "the request before cancellation").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    send_to_client(&server, &response_segment(invoke_id, 0, true, b"partial")).await;
    expect_segment_ack(&mut rx, invoke_id, 0).await;

    // Cancellation itself must not need the TSM lock: the dropped request
    // hands its cleanup to the dispatch task, which waits for the lock.
    let tsm_lock = client.tsm.lock();
    request.abort();
    let cancelled = timeout(Duration::from_secs(2), request)
        .await
        .expect("cancelling the request waited for the TSM lock");
    assert!(cancelled.unwrap_err().is_cancelled());
    drop(tsm_lock);

    assert!(
        timeout(
            Duration::from_secs(2),
            client.segmented_cleanup.wait_processed()
        )
        .await
        .expect("dispatch did not process cancellation cleanup"),
        "cancellation cleanup did not remove the matching reassembly"
    );
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 0);
    assert!(
        timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
        "cancellation cleanup sent an unexpected peer PDU"
    );

    send_to_client(&server, &response_segment(invoke_id, 1, true, b"late")).await;
    expect_client_abort(&mut rx, invoke_id, "a segment after caller cancellation").await;
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("task retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn caller_cancellation_reclaims_segmented_request_sender() {
    let config = ClientConfig {
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    };
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(BACnetClient::start(config, client_transport).await.unwrap());

    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });
    let invoke_id = match recv_apdu(&mut rx, "the segmented request before cancellation").await {
        Apdu::ConfirmedRequest(request) => {
            assert!(request.segmented);
            request.invoke_id
        }
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        !timeout(
            Duration::from_secs(2),
            client.segmented_cleanup.wait_processed()
        )
        .await
        .expect("dispatch did not process segmented-request cancellation"),
        "outgoing request unexpectedly owned receive reassembly state"
    );
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 0);
    assert!(
        !client
            .seg_ack_senders
            .lock()
            .contains_key(&(bacnet_types::MacAddr::from_slice(SERVER_MAC), invoke_id)),
        "cancellation left the SegmentAck sender registered"
    );
    assert!(
        timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
        "cancellation cleanup sent an unexpected peer PDU"
    );

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("task retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A set segment timeout, not the APDU timeout, paces a segmented request:
/// the unacknowledged first segment goes again one segment timeout after it
/// was sent (#2617).
#[tokio::test(start_paused = true)]
async fn a_set_segment_timeout_paces_a_segmented_request() {
    let config = ClientConfig {
        apdu_timeout_ms: 6_000,
        apdu_segment_timeout_ms: Some(250),
        apdu_retries: 1,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    };
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(BACnetClient::start(config, client_transport).await.unwrap());
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });
    let mut sent = Vec::new();
    for _ in 0..2 {
        match recv_apdu(&mut rx, "a request segment").await {
            Apdu::ConfirmedRequest(segment) => {
                assert_eq!(segment.sequence_number, Some(0));
                sent.push(tokio::time::Instant::now());
            }
            other => panic!("expected ConfirmedRequest segment, got {other:?}"),
        }
    }
    let gap = sent[1] - sent[0];
    assert!(
        gap >= Duration::from_millis(250) && gap < Duration::from_millis(300),
        "segment sent again after {gap:?}, expected the 250 ms segment timeout"
    );
    let result = request.await.unwrap();
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    let mut client =
        Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request task retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A set segment timeout also bounds the pause between an answer's
/// segments: four of them, whatever the APDU timeout (#2617).
#[tokio::test(start_paused = true)]
async fn a_set_segment_timeout_bounds_an_answers_pause() {
    let config = ClientConfig {
        apdu_timeout_ms: 6_000,
        apdu_segment_timeout_ms: Some(50),
        apdu_retries: 3,
        ..ClientConfig::default()
    };
    let (task, server, mut rx, invoke_id) = start_reassembly(config).await;
    send_to_client(&server, &response_segment(invoke_id, 0, true, &[0x01; 8])).await;
    expect_segment_ack(&mut rx, invoke_id, 0).await;
    let acked = tokio::time::Instant::now();
    let (mut client, result) = task.await.unwrap();
    let waited = acked.elapsed();
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert!(
        waited >= Duration::from_millis(200) && waited < Duration::from_millis(250),
        "the answer was given up {waited:?} after its last segment, expected 4 x 50 ms"
    );
    client.stop().await.unwrap();
}

/// A segment timeout of zero is refused at start; none takes the APDU
/// timeout.
#[tokio::test]
async fn a_zero_segment_timeout_is_refused() {
    let config = ClientConfig {
        apdu_segment_timeout_ms: Some(0),
        ..ClientConfig::default()
    };
    let (client_transport, _server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let error = BACnetClient::start(config, client_transport)
        .await
        .err()
        .expect("a zero segment timeout is refused");
    assert!(
        error.to_string().contains("apdu-segment-timeout"),
        "{error}"
    );
    assert_eq!(ClientConfig::default().segment_timeout_ms(), 6_000);
}
