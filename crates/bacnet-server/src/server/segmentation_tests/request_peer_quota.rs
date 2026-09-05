//! Owner-policy partition: 16 active incoming requests per existing SegKey peer.

use super::*;
use request_reassembly::{
    expect_positive_ack, inject_routed_segment, present_value, recv_apdu, send_apdu,
    send_segment_with_window, sent_routed_frame, split_into, start_reassembly_server,
    start_routed_reassembly_server, write_property_payload,
};

/// Consume a real wire reply, checking both routed and immediate destinations.
pub(super) async fn next_routed_apdu(
    sent: &SentFrames,
    index: &mut usize,
    router: &MacAddr,
    remote: &NpduAddress,
) -> Apdu {
    tokio::time::timeout(Duration::from_secs(2), async {
        while sent_count(sent) <= *index {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for routed reply");
    let (npdu, link_destination) = sent_routed_frame(sent, *index);
    *index += 1;
    assert_eq!(&link_destination, router);
    assert_eq!(npdu.destination.as_ref(), Some(remote));
    decode_apdu(npdu.payload).unwrap()
}

pub(super) fn assert_positive_ack(reply: Apdu, invoke_id: u8, seq: u8) {
    match reply {
        Apdu::SegmentAck(ack) => {
            assert!(ack.sent_by_server && !ack.negative_ack);
            assert_eq!(ack.invoke_id, invoke_id);
            assert_eq!(ack.sequence_number, seq);
            assert_eq!(ack.actual_window_size, 1);
        }
        other => panic!("expected positive SegmentAck, got {other:?}"),
    }
}

pub(super) fn assert_server_abort(reply: Apdu, invoke_id: u8, reason: AbortReason) {
    match reply {
        Apdu::Abort(abort) => {
            assert!(abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, reason);
        }
        other => panic!("expected server Abort {reason:?}, got {other:?}"),
    }
}

#[tokio::test]
async fn request_peer_quota_sixteen_admitted_seventeenth_refused_other_peer_admitted() {
    let (server, incoming, sent) = start_routed_reassembly_server().await;
    let router_a = test_mac(30);
    let router_b = test_mac(31);
    let remote = routed_address(400, 0x40);
    let other_peer = routed_address(400, 0x41);
    let mut index = 0;
    for invoke_id in 0..16 {
        let router = if invoke_id % 2 == 0 {
            &router_a
        } else {
            &router_b
        };
        inject_routed_segment(&incoming, router, &remote, invoke_id, 0, true, &[1]).await;
        assert_positive_ack(
            next_routed_apdu(&sent, &mut index, router, &remote).await,
            invoke_id,
            0,
        );
    }
    for invoke_id in 16..24 {
        let router = if invoke_id % 2 == 0 {
            &router_b
        } else {
            &router_a
        };
        inject_routed_segment(&incoming, router, &remote, invoke_id, 0, true, &[2]).await;
        assert_server_abort(
            next_routed_apdu(&sent, &mut index, router, &remote).await,
            invoke_id,
            AbortReason::OUT_OF_RESOURCES,
        );
    }
    inject_routed_segment(&incoming, &router_b, &other_peer, 16, 0, true, &[3]).await;
    assert_positive_ack(
        next_routed_apdu(&sent, &mut index, &router_b, &other_peer).await,
        16,
        0,
    );
    assert_eq!(present_value(&server).await, "");
    assert_eq!(
        sent_count(&sent),
        index,
        "no service response to partial requests"
    );
}

#[tokio::test]
async fn request_peer_quota_denial_is_stateless_and_active_transfers_complete() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "all-sixteen-survive-denial-and-duplicates";
    let chunks = split_into(&write_property_payload(text), 3);
    for invoke_id in 0..16 {
        send_segment_with_window(&client, invoke_id, 0, 1, true, &chunks[0]).await;
        expect_positive_ack(&mut rx, invoke_id, 0).await;
    }
    for invoke_id in 16..32 {
        // Even a complete service payload in a final initial segment cannot
        // evade admission, insert a receiver, or produce a service effect.
        send_segment_with_window(
            &client,
            invoke_id,
            0,
            1,
            false,
            &write_property_payload("denied-write"),
        )
        .await;
        assert_server_abort(
            recv_apdu(&mut rx, "quota denial").await,
            invoke_id,
            AbortReason::OUT_OF_RESOURCES,
        );
        send_segment_with_window(&client, invoke_id, 1, 1, false, &chunks[2]).await;
        assert_server_abort(
            recv_apdu(&mut rx, "denied invoke has no state").await,
            invoke_id,
            AbortReason::INVALID_APDU_IN_THIS_STATE,
        );
    }
    assert_eq!(present_value(&server).await, "");
    for invoke_id in 0..16 {
        send_segment_with_window(&client, invoke_id, 1, 1, true, &chunks[1]).await;
        expect_positive_ack(&mut rx, invoke_id, 1).await;
        send_segment_with_window(&client, invoke_id, 0, 1, true, &[0xEE]).await;
        assert!(
            matches!(recv_apdu(&mut rx, "duplicate zero at quota").await,
            Apdu::SegmentAck(ack) if ack.sent_by_server && ack.negative_ack
                && ack.invoke_id == invoke_id && ack.sequence_number == 1)
        );
    }
    for invoke_id in 0..16 {
        send_segment_with_window(&client, invoke_id, 2, 1, false, &chunks[2]).await;
        expect_positive_ack(&mut rx, invoke_id, 2).await;
        assert!(
            matches!(recv_apdu(&mut rx, "unaffected active request completes").await,
            Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id)
        );
        assert_eq!(present_value(&server).await, text);
        // Every completion releases one slot; refill it before the next one.
        send_segment_with_window(&client, 100 + invoke_id, 0, 1, true, &chunks[0]).await;
        expect_positive_ack(&mut rx, 100 + invoke_id, 0).await;
        send_segment_with_window(&client, 200, 0, 1, true, &chunks[0]).await;
        assert_server_abort(
            recv_apdu(&mut rx, "refilled quota").await,
            200,
            AbortReason::OUT_OF_RESOURCES,
        );
    }
    assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .is_err());
}

#[tokio::test]
async fn request_peer_quota_wrapped_zero_uses_existing_segment_cap_and_frees_slot() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    for invoke_id in 1..16 {
        send_segment_with_window(&client, invoke_id, 0, 1, true, &[1]).await;
        expect_positive_ack(&mut rx, invoke_id, 0).await;
    }
    for seq in 0..=255 {
        send_segment_with_window(&client, 0, seq, 1, true, &[2]).await;
        expect_positive_ack(&mut rx, 0, seq).await;
    }
    // This zero belongs to the existing 256-segment session, not admission.
    send_segment_with_window(&client, 0, 0, 1, true, &[3]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "wrapped zero at full peer quota").await,
        0,
        AbortReason::BUFFER_OVERFLOW,
    );
    send_segment_with_window(&client, 0, 1, 1, false, &[3]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "overflow removed session").await,
        0,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    send_segment_with_window(&client, 16, 0, 1, true, &[4]).await;
    expect_positive_ack(&mut rx, 16, 0).await;
    assert_eq!(present_value(&server).await, "");
}

#[tokio::test]
async fn request_peer_quota_abort_and_save_failure_release_only_owned_slots() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    for invoke_id in 0..16 {
        send_segment_with_window(&client, invoke_id, 0, 1, true, &[1]).await;
        expect_positive_ack(&mut rx, invoke_id, 0).await;
    }
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
        send_segment_with_window(&client, 16, 0, 1, true, &[2]).await;
        let reply = recv_apdu(&mut rx, "admission after directional Abort").await;
        if sent_by_server {
            assert_server_abort(reply, 16, AbortReason::OUT_OF_RESOURCES);
        } else {
            assert_positive_ack(reply, 16, 0);
        }
    }
    send_segment_with_window(&client, 0, 1, 1, true, &[2]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "peer Abort removed inbound").await,
        0,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );

    send_segment_with_window(&client, 1, 1, 1, true, &[0; 1477]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "failed next save").await,
        1,
        AbortReason::BUFFER_OVERFLOW,
    );
    send_segment_with_window(&client, 1, 2, 1, true, &[2]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "failed next save removed inbound").await,
        1,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    // Fifteen slots remain. Repeated failed first saves must not consume the last.
    for invoke_id in 17..20 {
        send_segment_with_window(&client, invoke_id, 0, 1, true, &[0; 1477]).await;
        assert_server_abort(
            recv_apdu(&mut rx, "failed initial save").await,
            invoke_id,
            AbortReason::BUFFER_OVERFLOW,
        );
        send_segment_with_window(&client, invoke_id, 1, 1, true, &[2]).await;
        assert_server_abort(
            recv_apdu(&mut rx, "failed first save has no state").await,
            invoke_id,
            AbortReason::INVALID_APDU_IN_THIS_STATE,
        );
    }
    send_segment_with_window(&client, 20, 0, 1, true, &[3]).await;
    expect_positive_ack(&mut rx, 20, 0).await;
    send_segment_with_window(&client, 21, 0, 1, true, &[0; 1477]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "quota before attempted first save").await,
        21,
        AbortReason::OUT_OF_RESOURCES,
    );
    assert_eq!(present_value(&server).await, "");
    assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .is_err());
}

async fn send_initial_with_raw_window(
    client: &bacnet_transport::loopback::LoopbackTransport,
    invoke_id: u8,
    window: u8,
) {
    use bacnet_encoding::npdu::{encode_npdu, Npdu};
    let mut buf = BytesMut::new();
    encode_apdu(
        &mut buf,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: true,
            more_follows: true,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: Some(0),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::from(vec![0; 1477]),
        }),
    )
    .unwrap();
    // The normal encoder rejects invalid windows. Inject a malformed wire field
    // like the existing segmentation_rx integration test, not a fake dispatch.
    buf[4] = window;
    let npdu = Npdu {
        payload: buf.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    client.send_unicast(&buf, &[0x02]).await.unwrap();
}

#[tokio::test]
async fn request_peer_quota_preserves_window_and_support_precedence() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    for invoke_id in 0..16 {
        send_segment_with_window(&client, invoke_id, 0, 1, true, &[1]).await;
        expect_positive_ack(&mut rx, invoke_id, 0).await;
    }
    for window in [0, 128, 255] {
        send_initial_with_raw_window(&client, 16, window).await;
        assert_server_abort(
            recv_apdu(&mut rx, "invalid window before quota").await,
            16,
            AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
        );
    }
    send_segment_with_window(&client, 16, 0, 127, true, &[1]).await;
    assert_server_abort(
        recv_apdu(&mut rx, "valid window reaches quota").await,
        16,
        AbortReason::OUT_OF_RESOURCES,
    );
    assert_eq!(present_value(&server).await, "");

    // Unsupported devices cannot accumulate inbound sessions: their immutable
    // advertisement rejects even invalid-window traffic before any admission.
    for segmentation in [Segmentation::NONE, Segmentation::TRANSMIT] {
        let (server, client, mut rx) = start_reassembly_server(segmentation).await;
        for invoke_id in 0..=16 {
            send_initial_with_raw_window(&client, invoke_id, 0).await;
            assert_server_abort(
                recv_apdu(&mut rx, "unsupported before window and quota").await,
                invoke_id,
                AbortReason::SEGMENTATION_NOT_SUPPORTED,
            );
        }
        assert_eq!(present_value(&server).await, "");
    }
}
