//! Receive SegmentTimer and independent local retention policy.
use super::*;
use request_reassembly::{
    expect_positive_ack, present_value, recv_apdu, send_segment_with_window, split_into,
    start_reassembly_server, write_property_payload,
};

#[tokio::test(start_paused = true)]
async fn default_receive_timeout_keeps_a_transfer_live_after_four_seconds() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let chunks = split_into(&write_property_payload("within-receive-tseg"), 2);
    send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 42, 0).await;
    tokio::time::advance(Duration::from_millis(4001)).await;
    send_segment_with_window(&client, 42, 1, 1, false, &chunks[1]).await;
    expect_positive_ack(&mut rx, 42, 1).await;
    assert!(matches!(recv_apdu(&mut rx, "completed request").await,
        Apdu::SimpleAck(ack) if ack.invoke_id == 42));
    assert_eq!(present_value(&server).await, "within-receive-tseg");
}

#[tokio::test(start_paused = true)]
async fn configured_receive_timeout_keeps_exact_equality_live_and_refreshes_deadline() {
    let (server, client, mut rx) =
        request_reassembly::start_reassembly_server_with_timeout(Segmentation::RECEIVE, 1000).await;
    let chunks = split_into(&write_property_payload("refreshed-at-equality"), 3);
    send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 42, 0).await;
    tokio::time::advance(Duration::from_secs(4)).await;
    send_segment_with_window(&client, 42, 1, 1, true, &chunks[1]).await;
    expect_positive_ack(&mut rx, 42, 1).await;
    // Cross the retired first deadline while the refreshed transfer is live.
    tokio::time::advance(Duration::from_secs(4)).await;
    send_segment_with_window(&client, 42, 2, 1, false, &chunks[2]).await;
    expect_positive_ack(&mut rx, 42, 2).await;
    assert!(
        matches!(recv_apdu(&mut rx, "completed equality request").await,
        Apdu::SimpleAck(ack) if ack.invoke_id == 42)
    );
    assert_eq!(present_value(&server).await, "refreshed-at-equality");
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    assert!(
        rx.try_recv().is_err(),
        "completed state has no stale expiry"
    );
}

#[tokio::test(start_paused = true)]
async fn quiet_protocol_expiry_releases_payload_without_an_abort_or_new_ingress() {
    use crate::server::segmented_receive::tests::observe_payload_drops;
    use std::sync::atomic::AtomicUsize;
    let (server, client, mut rx) =
        request_reassembly::start_reassembly_server_with_timeout(Segmentation::BOTH, 1000).await;
    let drops = Arc::new(AtomicUsize::new(0));
    let probe = observe_payload_drops(&drops);
    let chunks = split_into(&write_property_payload("fresh-incarnation"), 2);
    send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 42, 0).await;
    drop(probe);
    tokio::time::advance(Duration::from_secs(4)).await;
    tokio::task::yield_now().await;
    assert_eq!(drops.load(Ordering::SeqCst), 0, "equality remains live");
    // Tokio's timer wheel rounds the first strict nanosecond up to a tick.
    tokio::time::advance(Duration::from_millis(1)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "quiet dispatch reclaims ownership"
    );
    assert!(rx.try_recv().is_err(), "protocol expiry is silent");
    send_segment_with_window(&client, 42, 1, 1, false, &chunks[1]).await;
    request_peer_quota::assert_server_abort(
        recv_apdu(&mut rx, "orphan after expiry").await,
        42,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 42, 0).await;
    send_segment_with_window(&client, 42, 1, 1, false, &chunks[1]).await;
    expect_positive_ack(&mut rx, 42, 1).await;
    assert!(matches!(
        recv_apdu(&mut rx, "new incarnation").await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(present_value(&server).await, "fresh-incarnation");
}

#[tokio::test(start_paused = true)]
async fn quiet_progress_expiry_aborts_original_routed_next_hop_after_alternate_activity() {
    let (_server, incoming, sent) = request_reassembly::start_routed_reassembly_server().await;
    let original = test_mac(30);
    let alternate = test_mac(31);
    let remote = routed_address(400, 9);
    let mut index = 0;
    request_reassembly::inject_routed_segment(&incoming, &original, &remote, 42, 0, true, &[1])
        .await;
    request_peer_quota::assert_positive_ack(
        request_peer_quota::next_routed_apdu(&sent, &mut index, &original, &remote).await,
        42,
        0,
    );
    tokio::time::advance(Duration::from_secs(15)).await;
    request_reassembly::inject_routed_segment(&incoming, &alternate, &remote, 42, 2, true, &[2])
        .await;
    assert!(
        matches!(request_peer_quota::next_routed_apdu(&sent, &mut index, &alternate, &remote).await,
        Apdu::SegmentAck(ack) if ack.negative_ack)
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    request_peer_quota::assert_server_abort(
        request_peer_quota::next_routed_apdu(&sent, &mut index, &original, &remote).await,
        42,
        AbortReason::OTHER,
    );
    assert_eq!(sent_count(&sent), index);
}

#[tokio::test(start_paused = true)]
async fn stopping_during_local_expiry_abort_joins_dispatch_and_releases_payload() {
    use crate::server::segmented_receive::tests::observe_payload_drops;
    use std::sync::atomic::AtomicUsize;
    let (mut server, incoming, sent) = request_reassembly::start_routed_reassembly_server().await;
    let control = server.test_network().transport().handle();
    let drops = Arc::new(AtomicUsize::new(0));
    let probe = observe_payload_drops(&drops);
    let router = test_mac(30);
    let remote = routed_address(400, 9);
    request_reassembly::inject_routed_segment(&incoming, &router, &remote, 42, 0, true, &[1]).await;
    wait_for_sent_len(&sent, 1).await;
    drop(probe);
    control.block_next_send();
    tokio::time::advance(Duration::from_secs(16)).await;
    control.wait_blocked().await;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(2), server.stop())
        .await
        .unwrap()
        .unwrap();
    control.release_sends(1);
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(sent_count(&sent), 2, "no detached Abort work survives stop");
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "payload ownership released once"
    );
}

#[tokio::test(start_paused = true)]
async fn duplicate_and_gap_each_refresh_receive_timeout_without_saving_payload() {
    let (server, client, mut rx) =
        request_reassembly::start_reassembly_server_with_timeout(Segmentation::BOTH, 250).await;
    let chunks = split_into(&write_property_payload("only-original-bytes"), 2);
    send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 42, 0).await;
    for seq in [0, 2] {
        tokio::time::advance(Duration::from_millis(750)).await;
        send_segment_with_window(&client, 42, seq, 1, true, &[0xEE]).await;
        assert!(matches!(recv_apdu(&mut rx, "activity refresh NAK").await,
            Apdu::SegmentAck(ack) if ack.negative_ack && ack.invoke_id == 42 && ack.sequence_number == 0));
    }
    tokio::time::advance(Duration::from_millis(750)).await;
    send_segment_with_window(&client, 42, 1, 1, false, &chunks[1]).await;
    expect_positive_ack(&mut rx, 42, 1).await;
    assert!(matches!(
        recv_apdu(&mut rx, "refreshed request completed").await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(present_value(&server).await, "only-original-bytes");
}
