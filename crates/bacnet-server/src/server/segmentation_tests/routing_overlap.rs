use super::*;

#[tokio::test]
async fn routed_source_segment_ack_and_abort_match_across_immediate_routers() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let router_a = test_mac(16);
    let router_b = test_mac(17);
    let remote = routed_address(100, 0x10);
    let other_remote = routed_address(101, 0x20);
    let invoke_id = 0x50;
    let handle = spawn_segmented_complex_ack_from_network_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        Arc::clone(&seg_send_permits),
        SegmentedSendTestRequest {
            source_mac: router_a.clone(),
            source_network: Some(remote.clone()),
            invoke_id,
            service_ack_data: vec![0xF3; 128],
            options: SegmentedSendOptions {
                segment_timeout: Duration::from_millis(500),
                max_retries: 1,
            },
        },
    );

    wait_for_sent_len(&sent, 1).await;
    assert_eq!(complex_ack_sequence(&sent, 0), 0);
    assert_eq!(sent_link_destination(&sent, 0), router_a);
    assert_eq!(sent_npdu_destination(&sent, 0), Some(remote.clone()));

    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_b,
        Some(other_remote.clone()),
        Apdu::SegmentAck(segment_ack(invoke_id, true, 0)),
    )
    .await;
    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_b,
        Some(remote.clone()),
        Apdu::SegmentAck(segment_ack(invoke_id, false, 0)),
    )
    .await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(
        complex_ack_sequence(&sent, 1),
        1,
        "SegmentACK from a different NPDU source must not retransmit segment 0"
    );
    assert_eq!(sent_link_destination(&sent, 1), router_a);
    assert_eq!(sent_npdu_destination(&sent, 1), Some(remote.clone()));

    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_b,
        Some(other_remote),
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_b,
        Some(remote.clone()),
        Apdu::SegmentAck(segment_ack(invoke_id, true, 0)),
    )
    .await;

    wait_for_sent_len(&sent, 3).await;
    assert_eq!(
        complex_ack_sequence(&sent, 2),
        1,
        "matching routed negative SegmentACK should retransmit the current segment"
    );
    assert_eq!(sent_link_destination(&sent, 2), router_a);
    assert_eq!(sent_npdu_destination(&sent, 2), Some(remote.clone()));

    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_b,
        Some(remote),
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("matching routed client Abort should terminate segmented response task")
        .expect("segmented response task should not panic");
    assert_eq!(
        sent_count(&sent),
        3,
        "server must not send a timeout Abort after matching routed client Abort"
    );
    assert!(seg_ack_senders.lock().await.is_empty());
}

#[tokio::test]
async fn dispatch_ack_flood_does_not_drop_valid_segment_ack() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(11);
    let invoke_id = 0x4B;
    let handle = spawn_segmented_complex_ack(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac.clone(),
        invoke_id,
        vec![0xEF; 128],
    );

    wait_for_sent_len(&sent, 1).await;
    for _ in 0..32 {
        dispatch_test_apdu(
            &network,
            &seg_ack_senders,
            &source_mac,
            Apdu::SegmentAck(segment_ack(invoke_id, false, 1)),
        )
        .await;
    }
    dispatch_test_apdu(
        &network,
        &seg_ack_senders,
        &source_mac,
        Apdu::SegmentAck(segment_ack(invoke_id, true, 0)),
    )
    .await;

    wait_for_sent_len(&sent, 2).await;
    assert_eq!(complex_ack_sequence(&sent, 1), 1);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn overlapping_same_peer_invoke_id_cancels_old_segmented_sender() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let source_mac = test_mac(12);
    let invoke_id = 0x4C;
    let first = spawn_segmented_complex_ack_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac.clone(),
        invoke_id,
        vec![0xF0; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(50),
            max_retries: 0,
        },
    );

    wait_for_sent_len(&sent, 1).await;
    let second = spawn_segmented_complex_ack_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        source_mac,
        invoke_id,
        vec![0xF1; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_millis(500),
            max_retries: 0,
        },
    );

    wait_for_sent_len(&sent, 2).await;
    tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("older segmented sender should be cancelled")
        .expect("older segmented sender should not panic");
    tokio::time::sleep(Duration::from_millis(90)).await;
    assert!(
        (0..sent_count(&sent))
            .all(|index| !matches!(decoded_sent_apdu(&sent, index), Apdu::Abort(_))),
        "older segmented sender should not send timeout Abort after replacement"
    );

    second.abort();
    let _ = second.await;
}

#[tokio::test]
async fn routed_same_peer_replacement_cancels_old_sender_across_routers() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let router_a = test_mac(18);
    let router_b = test_mac(19);
    let remote = routed_address(102, 0x30);
    let invoke_id = 0x51;
    let options = SegmentedSendOptions {
        segment_timeout: Duration::from_secs(5),
        max_retries: 0,
    };
    let first = spawn_segmented_complex_ack_from_network_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        Arc::clone(&seg_send_permits),
        SegmentedSendTestRequest {
            source_mac: router_a.clone(),
            source_network: Some(remote.clone()),
            invoke_id,
            service_ack_data: vec![0xF4; 128],
            options,
        },
    );

    wait_for_sent_len(&sent, 1).await;
    let second = spawn_segmented_complex_ack_from_network_with_options(
        Arc::clone(&network),
        Arc::clone(&seg_ack_senders),
        seg_send_permits,
        SegmentedSendTestRequest {
            source_mac: router_b,
            source_network: Some(remote.clone()),
            invoke_id,
            service_ack_data: vec![0xF5; 128],
            options,
        },
    );

    wait_for_sent_len(&sent, 2).await;
    tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("new routed path should cancel the older canonical sender")
        .expect("older segmented sender should not panic");
    assert_eq!(
        seg_ack_senders.lock().await.len(),
        1,
        "older sender cleanup must retain its replacement"
    );

    dispatch_test_apdu_from_network(
        &network,
        &seg_ack_senders,
        &router_a,
        Some(remote),
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(1), second)
        .await
        .expect("Abort on either routed path should terminate the replacement")
        .expect("replacement segmented sender should not panic");
    assert!(seg_ack_senders.lock().await.is_empty());
    assert_eq!(sent_count(&sent), 2);
}
