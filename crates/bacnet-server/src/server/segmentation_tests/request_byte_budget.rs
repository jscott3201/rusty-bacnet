//! #527 owner policy: real dispatch at the full, private 4 MiB payload limit.

use super::*;
use bacnet_transport::{loopback::LoopbackTransport, port::ReceivedNpdu};
use request_peer_quota::assert_server_abort;
use request_reassembly::{
    expect_positive_ack, present_value, recv_apdu, send_apdu, send_segment_with_window,
    start_reassembly_server, write_property_payload,
};

// Independent acceptance oracle, not a reduced/configurable production limit.
const BUDGET: usize = 4 * 1024 * 1024;

async fn fill_budget(
    client: &LoopbackTransport,
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    bytes: usize,
) -> [u8; 12] {
    let mut next = [0; 12];
    let mut remaining = bytes;
    let mut turn = 0;
    while remaining != 0 {
        // Round robin accepted progress keeps all twelve live, without pruning
        // or manipulating the production map or its 4s/16s timers.
        let peer = turn % 12;
        let size = remaining.min(1476);
        send_segment_with_window(
            client,
            peer as u8,
            next[peer],
            1,
            true,
            &vec![peer as u8; size],
        )
        .await;
        expect_positive_ack(rx, peer as u8, next[peer]).await;
        next[peer] += 1;
        remaining -= size;
        turn += 1;
    }
    next
}

#[tokio::test]
async fn request_byte_budget_repeated_initial_and_final_refusals_are_stateless() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let next = fill_budget(&client, &mut rx, BUDGET).await;
    let write = write_property_payload("must not execute a refused final initial request");
    for invoke in 12..20 {
        let (more, data) = if invoke % 2 == 0 {
            (true, &[1][..])
        } else {
            (false, &write[..])
        };
        send_segment_with_window(&client, invoke, 0, 1, more, data).await;
        assert_server_abort(
            recv_apdu(&mut rx, "initial budget refusal").await,
            invoke,
            AbortReason::BUFFER_OVERFLOW,
        );
        send_segment_with_window(&client, invoke, 1, 1, false, &[]).await;
        assert_server_abort(
            recv_apdu(&mut rx, "refusal created no state").await,
            invoke,
            AbortReason::INVALID_APDU_IN_THIS_STATE,
        );
    }
    for (invoke, seq) in next.into_iter().enumerate() {
        // Unrelated requests retain their expected sequence; zero-size NEW
        // segments consume count/progress but no payload bytes at exact fit.
        send_segment_with_window(&client, invoke as u8, seq, 1, true, &[]).await;
        expect_positive_ack(&mut rx, invoke as u8, seq).await;
    }
    assert_eq!(present_value(&server).await, "");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_repeated_final_continuation_denial_releases_only_current() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let write = write_property_payload("never write a partial request");
    let (prefix, final_byte) = write.split_at(write.len() - 1);
    let next = fill_budget(&client, &mut rx, BUDGET - prefix.len()).await;
    for invoke in 12..16 {
        // Each previous refusal must release precisely enough for this prefix.
        send_segment_with_window(&client, invoke, 0, 1, true, prefix).await;
        expect_positive_ack(&mut rx, invoke, 0).await;
        send_segment_with_window(&client, invoke, 1, 1, false, final_byte).await;
        assert_server_abort(
            recv_apdu(&mut rx, "one byte final continuation overflow").await,
            invoke,
            AbortReason::BUFFER_OVERFLOW,
        );
        send_segment_with_window(&client, invoke, 1, 1, false, final_byte).await;
        assert_server_abort(
            recv_apdu(&mut rx, "overflow removed only current request").await,
            invoke,
            AbortReason::INVALID_APDU_IN_THIS_STATE,
        );
        assert_eq!(present_value(&server).await, "");
    }
    for (invoke, seq) in next.into_iter().enumerate() {
        send_segment_with_window(&client, invoke as u8, seq, 1, true, &[]).await;
        expect_positive_ack(&mut rx, invoke as u8, seq).await;
    }
    // The other twelve retained their payload bytes, not just their slots.
    send_segment_with_window(&client, 16, 0, 1, false, &write).await;
    assert_server_abort(
        recv_apdu(&mut rx, "unrelated byte ownership survived").await,
        16,
        AbortReason::BUFFER_OVERFLOW,
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_completion_at_exact_fit_releases_capacity_for_initial_final() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "exact-fit-write";
    let write = write_property_payload(text);
    fill_budget(&client, &mut rx, BUDGET - write.len()).await;
    let (first, last) = write.split_at(1);
    send_segment_with_window(&client, 12, 0, 1, true, first).await;
    expect_positive_ack(&mut rx, 12, 0).await;
    assert_eq!(present_value(&server).await, "");
    send_segment_with_window(&client, 12, 1, 1, false, last).await;
    expect_positive_ack(&mut rx, 12, 1).await;
    assert!(
        matches!(recv_apdu(&mut rx, "completed exact fit").await, Apdu::SimpleAck(ack) if ack.invoke_id == 12)
    );
    assert_eq!(present_value(&server).await, text);
    for (invoke, value) in [(13, "new-exact-write"), (14, "under-budget")] {
        let payload = write_property_payload(value);
        assert!(payload.len() <= write.len());
        send_segment_with_window(&client, invoke, 0, 1, false, &payload).await;
        expect_positive_ack(&mut rx, invoke, 0).await;
        assert!(
            matches!(recv_apdu(&mut rx, "initial final completion").await, Apdu::SimpleAck(ack) if ack.invoke_id == invoke)
        );
        assert_eq!(present_value(&server).await, value);
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_full_discards_oversized_duplicates_gaps_and_accepts_empty_growth() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let next = fill_budget(&client, &mut rx, BUDGET).await;
    for seq in [next[0] - 1, next[0] + 1, 0] {
        send_segment_with_window(&client, 0, seq, 1, false, &[7; 1500]).await;
        match recv_apdu(&mut rx, "discard before budget or validation").await {
            Apdu::SegmentAck(ack) => {
                assert!(ack.negative_ack && ack.sent_by_server);
                assert_eq!(ack.invoke_id, 0);
                assert_eq!(ack.sequence_number, next[0] - 1);
            }
            other => panic!("expected NAK for discarded input, got {other:?}"),
        }
    }
    // Zero-size acceptance at the full byte budget still reaches the existing
    // 256-segment limit. The wrapped NEW segment must not overwrite saved seq0.
    for seq in next[0]..=255 {
        send_segment_with_window(&client, 0, seq, 1, true, &[]).await;
        expect_positive_ack(&mut rx, 0, seq).await;
    }
    send_segment_with_window(&client, 0, 0, 1, true, &[]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "zero-size segment 257").await,
        0,
        AbortReason::BUFFER_OVERFLOW,
    );
    send_segment_with_window(&client, 12, 0, 1, true, &[1]).await;
    expect_positive_ack(&mut rx, 12, 0).await;
    assert_eq!(present_value(&server).await, "");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_server_abort_preserves_bytes_peer_abort_releases_them() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    fill_budget(&client, &mut rx, BUDGET).await;
    for sent_by_server in [true, false] {
        send_apdu(
            &client,
            &Apdu::Abort(AbortPdu {
                sent_by_server,
                invoke_id: 0,
                abort_reason: AbortReason::OTHER,
            }),
        )
        .await;
        send_segment_with_window(&client, 12, 0, 1, true, &[1]).await;
        let reply = recv_apdu(&mut rx, "capacity after inbound Abort").await;
        if sent_by_server {
            assert_server_abort(reply, 12, AbortReason::BUFFER_OVERFLOW);
        } else {
            request_peer_quota::assert_positive_ack(reply, 12, 0);
        }
    }
    assert_eq!(present_value(&server).await, "");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_is_per_server_instance_not_shared_globally() {
    let (mut full, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    fill_budget(&client, &mut rx, BUDGET).await;
    let (mut other, other_client, mut other_rx) = start_reassembly_server(Segmentation::BOTH).await;
    let write = write_property_payload("independent server budget");
    send_segment_with_window(&other_client, 0, 0, 1, false, &write).await;
    expect_positive_ack(&mut other_rx, 0, 0).await;
    assert!(
        matches!(recv_apdu(&mut other_rx, "other server completion").await, Apdu::SimpleAck(ack) if ack.invoke_id == 0)
    );
    assert_eq!(present_value(&other).await, "independent server budget");
    send_segment_with_window(&client, 12, 0, 1, true, &[1]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "first server stays full").await,
        12,
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!(present_value(&full).await, "");
    other.stop().await.unwrap();
    full.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_full_silent_oversized_duplicates_preserve_window_and_payload() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "full-budget window survives discarded payloads";
    let write = write_property_payload(text);
    fill_budget(&client, &mut rx, BUDGET - write.len()).await;
    send_segment_with_window(&client, 12, 0, 3, true, &write[..1]).await;
    send_segment_with_window(&client, 12, 1, 3, true, &write[1..]).await;
    // Two saved segments, full bytes, but the three-segment window is not full.
    // Its three-duplicate allowance must silently discard even oversized input.
    for _ in 0..3 {
        send_segment_with_window(&client, 12, 0, 3, false, &[9; 1500]).await;
    }
    send_segment_with_window(&client, 12, 7, 3, false, &[8; 1500]).await;
    match recv_apdu(&mut rx, "gap after silent duplicate allowance").await {
        Apdu::SegmentAck(ack) => {
            assert!(ack.negative_ack && ack.sent_by_server);
            assert_eq!(ack.invoke_id, 12);
            assert_eq!(ack.sequence_number, 1);
            assert_eq!(ack.actual_window_size, 3);
        }
        other => panic!("expected gap NAK, got {other:?}"),
    }
    assert_eq!(present_value(&server).await, "");
    send_segment_with_window(&client, 12, 2, 3, false, &[]).await;
    expect_positive_ack(&mut rx, 12, 2).await;
    assert!(
        matches!(recv_apdu(&mut rx, "byte-exact completion after discarded traffic").await, Apdu::SimpleAck(ack) if ack.invoke_id == 12)
    );
    assert_eq!(present_value(&server).await, text);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_byte_budget_actual_four_mib_exact_fit_then_one_byte_initial_abort() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    assert_eq!(2841 * 1476 + 988, BUDGET);
    let next = fill_budget(&client, &mut rx, BUDGET).await;
    assert_eq!(
        next,
        [237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 236, 236]
    );
    send_segment_with_window(&client, 12, 0, 1, true, &[1]).await;
    let reply = recv_apdu(&mut rx, "one byte beyond actual 4 MiB").await;
    let value = present_value(&server).await;
    server.stop().await.unwrap();
    assert_eq!(value, "", "incomplete requests must not write anything");
    assert_server_abort(reply, 12, AbortReason::BUFFER_OVERFLOW);
}
