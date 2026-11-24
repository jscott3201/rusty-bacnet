//! Duplicate classification while receiving segmented ConfirmedRequests (#383).

use super::*;
use request_reassembly::{
    expect_positive_ack, present_value, recv_apdu, send_segment_with_window, split_into,
    start_reassembly_server, write_property_payload,
};
use tokio::time::timeout;

async fn expect_negative_ack(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    invoke_id: u8,
    sequence_number: u8,
    window_size: u8,
    context: &str,
) {
    match recv_apdu(rx, context).await {
        Apdu::SegmentAck(ack) => {
            assert!(ack.negative_ack, "{context}");
            assert!(ack.sent_by_server, "{context}: server ACK role");
            assert_eq!(ack.invoke_id, invoke_id, "{context}");
            assert_eq!(ack.sequence_number, sequence_number, "{context}");
            assert_eq!(ack.actual_window_size, window_size, "{context}");
        }
        other => panic!("{context}: expected SegmentAck, got {other:?}"),
    }
}

async fn expect_silence(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    context: &str,
) {
    match timeout(Duration::from_millis(100), rx.recv()).await {
        Err(_) => {}
        Ok(Some(frame)) => {
            let npdu = decode_npdu(frame.npdu).unwrap();
            let apdu = decode_apdu(npdu.payload).unwrap();
            panic!("{context}: expected silence, got {apdu:?}");
        }
        Ok(None) => panic!("{context}: channel closed"),
    }
}

#[tokio::test(start_paused = true)]
async fn incomplete_window_silences_ndup_duplicates_then_naks_the_next() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::SEGMENTED_BOTH).await;
    let text = "duplicate-window";
    let chunks = split_into(&write_property_payload(text), 9);

    // Segment zero is acknowledged at once with the window (Clause
    // 5.4.5.1); each group of three counts from that acknowledgment (#2742).
    send_segment_with_window(&client, 25, 0, 3, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 25, 0).await;
    send_segment_with_window(&client, 25, 1, 3, true, &chunks[1]).await;
    send_segment_with_window(&client, 25, 0, 3, true, &[0xEE]).await;
    expect_silence(&mut rx, "duplicate before window boundary").await;
    send_segment_with_window(&client, 25, 2, 3, true, &chunks[2]).await;
    expect_silence(&mut rx, "inside the first group after segment zero").await;
    send_segment_with_window(&client, 25, 3, 3, true, &chunks[3]).await;
    expect_positive_ack(&mut rx, 25, 3).await;

    send_segment_with_window(&client, 25, 4, 3, true, &chunks[4]).await;
    for _ in 0..3 {
        send_segment_with_window(&client, 25, 4, 3, true, &[0xEE]).await;
    }
    expect_silence(&mut rx, "duplicates after window-boundary reset").await;

    send_segment_with_window(&client, 25, 4, 3, true, &[0xEE]).await;
    expect_negative_ack(&mut rx, 25, 4, 3, "fourth duplicate NAK").await;

    // Server TooManyDuplicateSegmentsReceived moves the duplicate baseline to
    // LastSequenceNumber, so the same last segment is no longer in-window.
    send_segment_with_window(&client, 25, 4, 3, true, &[0xEE]).await;
    expect_negative_ack(&mut rx, 25, 4, 3, "post-threshold baseline NAK").await;

    // The next group counts from the negative acknowledgment.
    send_segment_with_window(&client, 25, 5, 3, true, &chunks[5]).await;
    send_segment_with_window(&client, 25, 6, 3, true, &chunks[6]).await;
    expect_silence(&mut rx, "inside the group after the negative ack").await;
    send_segment_with_window(&client, 25, 7, 3, true, &chunks[7]).await;
    expect_positive_ack(&mut rx, 25, 7).await;
    send_segment_with_window(&client, 25, 7, 3, true, &[0xEE]).await;
    expect_negative_ack(&mut rx, 25, 7, 3, "post-boundary baseline NAK").await;

    send_segment_with_window(&client, 25, 8, 3, false, &chunks[8]).await;
    expect_positive_ack(&mut rx, 25, 8).await;
    match recv_apdu(&mut rx, "response after duplicate traffic").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, 25),
        other => panic!("expected SimpleAck, got {other:?}"),
    }
    assert_eq!(present_value(&server).await, text);
}

#[tokio::test(start_paused = true)]
async fn out_of_order_segment_naks_immediately_and_resets_duplicate_state() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::SEGMENTED_BOTH).await;
    let text = "gap-window";
    let chunks = split_into(&write_property_payload(text), 8);

    send_segment_with_window(&client, 27, 0, 4, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 27, 0).await;
    send_segment_with_window(&client, 27, 1, 4, true, &chunks[1]).await;
    send_segment_with_window(&client, 27, 0, 4, true, &[0xEE]).await;
    expect_silence(&mut rx, "duplicate before gap").await;

    send_segment_with_window(&client, 27, 3, 4, true, &[0xDD]).await;
    expect_negative_ack(&mut rx, 27, 1, 4, "gap NAK").await;

    send_segment_with_window(&client, 27, 2, 4, true, &chunks[2]).await;
    for _ in 0..4 {
        send_segment_with_window(&client, 27, 2, 4, true, &[0xEE]).await;
    }
    expect_silence(&mut rx, "duplicates after gap reset").await;
    send_segment_with_window(&client, 27, 2, 4, true, &[0xEE]).await;
    expect_negative_ack(&mut rx, 27, 2, 4, "post-gap threshold NAK").await;

    // The group counts from the last negative acknowledgment: 2 + 4
    // (Clause 5.4.5.2), not from where the previous group stood.
    for seq in 3..6 {
        send_segment_with_window(&client, 27, seq, 4, true, &chunks[usize::from(seq)]).await;
    }
    expect_silence(&mut rx, "inside the group after the negative ack").await;
    send_segment_with_window(&client, 27, 6, 4, true, &chunks[6]).await;
    expect_positive_ack(&mut rx, 27, 6).await;
    send_segment_with_window(&client, 27, 6, 4, true, &[0xEE]).await;
    expect_negative_ack(&mut rx, 27, 6, 4, "post-boundary baseline NAK").await;

    send_segment_with_window(&client, 27, 7, 4, false, &chunks[7]).await;
    expect_positive_ack(&mut rx, 27, 7).await;
    match recv_apdu(&mut rx, "response after gap").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, 27),
        other => panic!("expected SimpleAck, got {other:?}"),
    }
    assert_eq!(present_value(&server).await, text);
}
