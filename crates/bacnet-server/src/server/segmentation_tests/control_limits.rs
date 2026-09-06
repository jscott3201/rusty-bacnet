use super::*;

#[tokio::test]
async fn non_rung_request_header_conservatively_bounds_server_response() {
    let request = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(5),
        max_apdu_length: 50,
        invoke_id: 0x49,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        service_request: Bytes::new(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &request).unwrap();
    let client_max_segments = match decode_apdu(encoded.freeze()).unwrap() {
        Apdu::ConfirmedRequest(request) => {
            assert_eq!(request.max_segments, Some(4));
            request.max_segments
        }
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };

    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let source_mac = test_mac(9);

    BACnetServer::<RecordingTransport>::send_segmented_complex_ack(
        &network,
        &seg_ack_senders,
        &seg_send_permits,
        source_mac.as_slice(),
        None,
        0x49,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &[0xA5; 256],
        50,
        client_max_segments,
    )
    .await;

    assert_eq!(sent_count(&sent), 1);
    assert_eq!(abort_reason(&sent, 0), AbortReason::BUFFER_OVERFLOW);
    assert!(seg_ack_senders.lock().await.is_empty());
}

#[tokio::test]
async fn client_abort_routed_by_dispatch_terminates_segmented_complex_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(10);
    let invoke_id = 0x4A;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac.clone(),
        invoke_id,
        vec![0xEE; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(50),
            max_retries: 0,
        },
    );

    wait_for_sent_len(&sent, 1).await;
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("client Abort should terminate segmented response task")
        .expect("segmented response task should not panic");
    assert_eq!(
        sent_count(&sent),
        1,
        "server must not send a timeout Abort after client Abort"
    );
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "SegmentAck sender entry should be removed after client Abort"
    );
}

#[tokio::test]
async fn dispatch_accepts_segment_ack_before_send_future_returns() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let first_send_started = Arc::new(Notify::new());
    let release_first_send = Arc::new(Notify::new());
    let network = Arc::new(NetworkLayer::new(BlockingSendTransport::new(
        StdArc::clone(&sent),
        Arc::clone(&first_send_started),
        Arc::clone(&release_first_send),
    )));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let source_mac = test_mac(13);
    let invoke_id = 0x4D;
    let handle = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let source_mac = source_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                source_mac.as_slice(),
                None,
                invoke_id,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &[0xF2; 128],
                50,
                None,
                SegmentedSendOptions {
                    segment_timeout: Duration::from_millis(500),
                    max_retries: 0,
                },
            )
            .await;
        })
    };

    tokio::time::timeout(Duration::from_secs(1), first_send_started.notified())
        .await
        .expect("first segment send should start");
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::SegmentAck(segment_ack(invoke_id, false, 0)),
    )
    .await;
    release_first_send.notify_waiters();

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 1);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn client_abort_is_prioritized_over_queued_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let first_send_started = Arc::new(Notify::new());
    let release_first_send = Arc::new(Notify::new());
    let network = Arc::new(NetworkLayer::new(BlockingSendTransport::new(
        StdArc::clone(&sent),
        Arc::clone(&first_send_started),
        Arc::clone(&release_first_send),
    )));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let source_mac = test_mac(14);
    let invoke_id = 0x4E;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let source_mac = source_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                source_mac.as_slice(),
                None,
                invoke_id,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &[0xF3; 128],
                50,
                None,
                SegmentedSendOptions {
                    segment_timeout: Duration::from_millis(500),
                    max_retries: 0,
                },
            )
            .await;
        })
    };

    tokio::time::timeout(Duration::from_secs(1), first_send_started.notified())
        .await
        .expect("first segment send should start");
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::SegmentAck(segment_ack(invoke_id, false, 0)),
    )
    .await;
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    release_first_send.notify_waiters();

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("client Abort should terminate segmented response task")
        .expect("segmented response task should not panic");
    assert_eq!(
        sent_count(&sent),
        1,
        "queued SegmentACK must not advance after client Abort"
    );
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "SegmentAck sender entry should be removed after prioritized client Abort"
    );
}

#[tokio::test]
async fn same_key_cancel_is_prioritized_over_queued_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let first_send_started = Arc::new(Notify::new());
    let release_first_send = Arc::new(Notify::new());
    let network = Arc::new(NetworkLayer::new(BlockingSendTransport::new(
        StdArc::clone(&sent),
        Arc::clone(&first_send_started),
        Arc::clone(&release_first_send),
    )));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let source_mac = test_mac(15);
    let invoke_id = 0x4F;
    let first = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let source_mac = source_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                source_mac.as_slice(),
                None,
                invoke_id,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &[0xF4; 128],
                50,
                None,
                SegmentedSendOptions {
                    segment_timeout: Duration::from_millis(500),
                    max_retries: 0,
                },
            )
            .await;
        })
    };

    tokio::time::timeout(Duration::from_secs(1), first_send_started.notified())
        .await
        .expect("first segment send should start");
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::SegmentAck(segment_ack(invoke_id, false, 0)),
    )
    .await;

    let second = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let source_mac = source_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                source_mac.as_slice(),
                None,
                invoke_id,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &[0xF5; 128],
                50,
                None,
                SegmentedSendOptions {
                    segment_timeout: Duration::from_millis(500),
                    max_retries: 0,
                },
            )
            .await;
        })
    };

    wait_for_sent_len(&sent, 2).await;
    release_first_send.notify_waiters();
    tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("older segmented sender should be cancelled")
        .expect("older segmented sender should not panic");
    assert_eq!(
        sent_count(&sent),
        2,
        "queued SegmentACK must not advance after same-key Cancel"
    );

    second.abort();
    let _ = second.await;
}

#[tokio::test]
async fn same_key_replacement_is_rejected_when_live_sender_permits_are_exhausted() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let first_send_started = Arc::new(Notify::new());
    let release_first_send = Arc::new(Notify::new());
    let network = Arc::new(NetworkLayer::new(BlockingSendTransport::new(
        StdArc::clone(&sent),
        Arc::clone(&first_send_started),
        Arc::clone(&release_first_send),
    )));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(1));
    let source_mac = test_mac(16);
    let invoke_id = 0x50;
    let key: SegKey = (source_mac.clone(), None, invoke_id);

    let first = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let source_mac = source_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                source_mac.as_slice(),
                None,
                invoke_id,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &[0xF8; 128],
                50,
                None,
                SegmentedSendOptions {
                    segment_timeout: Duration::from_millis(500),
                    max_retries: 0,
                },
            )
            .await;
        })
    };

    tokio::time::timeout(Duration::from_secs(1), first_send_started.notified())
        .await
        .expect("first segment send should start");

    BACnetServer::<BlockingSendTransport>::send_segmented_complex_ack_with_options(
        &network,
        &seg_ack_senders,
        &seg_send_permits,
        source_mac.as_slice(),
        None,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &[0xF9; 128],
        50,
        None,
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(500),
            max_retries: 0,
        },
    )
    .await;

    assert_eq!(sent_count(&sent), 2);
    assert_eq!(abort_reason(&sent, 1), AbortReason::BUFFER_OVERFLOW);
    assert!(
        seg_ack_senders.lock().await.contains_key(&key),
        "rejected replacement must leave the original sender registered"
    );

    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    release_first_send.notify_waiters();
    tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("original sender should terminate after client Abort")
        .expect("original sender should not panic");
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "original sender should clean up after client Abort"
    );
}

#[tokio::test]
async fn segmented_complex_ack_rejects_new_sender_when_active_sender_limit_reached() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));

    {
        let mut senders = seg_ack_senders.lock().await;
        for idx in 0..MAX_SEG_SENDERS {
            let (handle, _segment_rx, _control_rx) = fake_segmented_send_handle(1, 2, 0);
            senders.insert((test_mac(idx as u8), None, idx as u8), handle);
        }
    }

    let source_mac = test_mac(200);
    let invoke_id = 0x60;
    BACnetServer::<RecordingTransport>::send_segmented_complex_ack(
        &network,
        &seg_ack_senders,
        &seg_send_permits,
        source_mac.as_slice(),
        None,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &[0xF6; 128],
        50,
        None,
    )
    .await;

    assert_eq!(sent_count(&sent), 1);
    assert_eq!(abort_reason(&sent, 0), AbortReason::BUFFER_OVERFLOW);
    assert_eq!(seg_ack_senders.lock().await.len(), MAX_SEG_SENDERS);
}

#[tokio::test]
async fn dispatch_returns_when_segment_ack_queue_is_full() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(13);
    let invoke_id = 0x4D;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let (handle, _rx, _control_rx) = fake_segmented_send_handle(1, 2, 0);
    handle
        .segment_ack_tx
        .try_send(segment_ack(invoke_id, false, 0))
        .expect("test queue should accept one SegmentACK");
    seg_ack_senders.lock().await.insert(key, handle);

    tokio::time::timeout(
        Duration::from_millis(50),
        dispatch_test_apdu(
            &network,
            &seg_ack_senders,
            &source_mac,
            Apdu::SegmentAck(segment_ack(invoke_id, false, 0)),
        ),
    )
    .await
    .expect("dispatch must not wait for capacity on a full SegmentACK queue");
}

#[tokio::test]
async fn dispatch_client_abort_not_blocked_by_full_segment_ack_queue() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(14);
    let invoke_id = 0x4E;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let (handle, _rx, mut control_rx) = fake_segmented_send_handle(1, 2, 0);
    handle
        .segment_ack_tx
        .try_send(segment_ack(invoke_id, false, 0))
        .expect("test queue should accept one SegmentACK");
    seg_ack_senders.lock().await.insert(key, handle);

    tokio::time::timeout(
        Duration::from_millis(50),
        dispatch_test_apdu(
            &network,
            &seg_ack_senders,
            &source_mac,
            Apdu::Abort(AbortPdu {
                sent_by_server: false,
                invoke_id,
                abort_reason: AbortReason::OTHER,
            }),
        ),
    )
    .await
    .expect("client Abort dispatch must not wait for SegmentACK queue capacity");

    tokio::time::timeout(Duration::from_millis(50), control_rx.changed())
        .await
        .expect("Abort control event should be delivered")
        .expect("Abort control channel should remain open");
    let control_event = control_rx.borrow_and_update().clone();
    match control_event {
        Some(SegmentedSendControlEvent::Abort(abort)) => {
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, AbortReason::OTHER);
        }
        other => panic!("expected Abort control event, got {other:?}"),
    }
}

#[tokio::test]
async fn same_key_replacement_does_not_wait_for_full_old_queue() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(15);
    let invoke_id = 0x4F;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let (old_handle, _old_rx, mut old_control_rx) = fake_segmented_send_handle(1, 2, 0);
    old_handle
        .segment_ack_tx
        .try_send(segment_ack(invoke_id, false, 0))
        .expect("test queue should accept one SegmentACK");
    seg_ack_senders.lock().await.insert(key, old_handle);

    let replacement = spawn_segmented_complex_ack_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xF2; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(500),
            max_retries: 0,
        },
    );

    wait_for_sent_len(&sent, 1).await;
    tokio::time::timeout(Duration::from_millis(50), old_control_rx.changed())
        .await
        .expect("replacement should deliver a nonblocking Cancel")
        .expect("old control channel should remain open");
    assert!(matches!(
        old_control_rx.borrow_and_update().clone(),
        Some(SegmentedSendControlEvent::Cancel)
    ));

    replacement.abort();
    let _ = replacement.await;
}
