use super::*;

#[tokio::test]
async fn segmented_complex_ack_retransmits_segment_zero_after_negative_ack_zero() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(1);
    let invoke_id = 0x41;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0x55; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 0);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn segmented_complex_ack_ignores_future_positive_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(2);
    let invoke_id = 0x42;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0x66; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 1)).await;
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 0);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn segmented_complex_ack_retransmits_current_segment_after_negative_ack_previous_sequence() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(4);
    let invoke_id = 0x44;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0x88; 192],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 1);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 0)).await;

    wait_for_sent_len(&sent, 3).await;
    assert_eq!(complex_ack_sequence(&sent, 2), 1);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 1)).await;

    wait_for_sent_len(&sent, 4).await;
    assert_eq!(complex_ack_sequence(&sent, 3), 2);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn segmented_complex_ack_ignores_out_of_range_negative_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(5);
    let invoke_id = 0x45;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0x99; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 255)).await;
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 1);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn segmented_complex_ack_ignores_segment_ack_with_server_bit_set() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(6);
    let invoke_id = 0x46;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xAA; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    send_segment_ack(
        &seg_ack_senders,
        &key,
        server_segment_ack(invoke_id, false, 0),
    )
    .await;
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 0);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn segmented_complex_ack_ignores_stale_positive_final_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(7);
    let invoke_id = 0x47;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xBB; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 0)).await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 1);
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 1)).await;

    wait_for_sent_len(&sent, 3).await;
    assert_eq!(complex_ack_sequence(&sent, 2), 2);
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 1)).await;
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 1)).await;

    wait_for_sent_len(&sent, 4).await;
    assert_eq!(complex_ack_sequence(&sent, 3), 2);
    send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, false, 2)).await;

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("segmented response task should exit after final ACK")
        .expect("segmented response task should not panic");
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "SegmentAck sender entry should be removed after final ACK"
    );
}

#[tokio::test]
async fn segmented_complex_ack_aborts_after_repeated_negative_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(3);
    let invoke_id = 0x43;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0x77; 128],
    );

    wait_for_sent_len(&sent, 1).await;

    for expected_len in 2..=(MAX_NEG_SEGMENT_ACK_RETRIES as usize + 2) {
        send_segment_ack(&seg_ack_senders, &key, segment_ack(invoke_id, true, 0)).await;
        wait_for_sent_len(&sent, expected_len).await;
    }

    assert_eq!(
        abort_reason(&sent, sent_count(&sent) - 1),
        AbortReason::TSM_TIMEOUT
    );

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("segmented response task should exit after abort")
        .expect("segmented response task should not panic");
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "SegmentAck sender entry should be removed after abort"
    );
}

#[tokio::test]
async fn segmented_complex_ack_retransmits_after_segment_ack_timeout_then_goes_idle() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(8);
    let invoke_id = 0x48;
    let key: SegKey = (source_mac.clone(), None, invoke_id);
    let handle = spawn_segmented_complex_ack_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xCC; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(25),
            max_retries: 1,
        },
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 0);

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("segmented response task should exit after timeout retry exhaustion")
        .expect("segmented response task should not panic");
    assert_eq!(
        sent_count(&sent),
        2,
        "server should go idle after final segment timeout without sending Abort"
    );
    assert!(
        (0..sent_count(&sent))
            .all(|index| !matches!(decoded_sent_apdu(&sent, index), Apdu::Abort(_))),
        "server must not send Abort after final segment timeout"
    );
    assert!(
        !seg_ack_senders.lock().await.contains_key(&key),
        "SegmentAck sender entry should be removed after timeout retry exhaustion"
    );
}

#[tokio::test]
async fn segmented_complex_ack_sets_npdu_expecting_reply_for_segments() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(9);
    let invoke_id = 0x49;
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xDD; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    assert!(sent_expecting_reply(&sent, 0));

    handle.abort();
    let _ = handle.await;
}
