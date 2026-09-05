//! Private defensive no-progress cleanup (#527), not a SegmentTimer transition.

use super::*;
use request_reassembly::{
    expect_positive_ack, present_value, recv_apdu, send_segment_with_window, split_into,
    start_reassembly_server, write_property_payload,
};
use tokio::time::{sleep_until, timeout, Instant as TokioInstant};

#[tokio::test]
async fn request_progress_reclaims_128_stalled_slots_before_admission() {
    timeout(Duration::from_secs(25), async {
        let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
        let discarded = split_into(&write_property_payload("must-not-be-written"), 2);
        let initial_value = present_value(&server).await;
        for invoke_id in 0..128 {
            send_segment_with_window(&client, invoke_id, 0, 1, true, &discarded[0]).await;
            expect_positive_ack(&mut rx, invoke_id, 0).await;
        }
        let started = TokioInstant::now();
        let admitted = split_into(&write_property_payload("admitted-after-cleanup"), 2);
        send_segment_with_window(&client, 128, 0, 1, true, &admitted[0]).await;
        match recv_apdu(&mut rx, "all 128 slots occupied").await {
            Apdu::Abort(abort) => {
                assert!(abort.sent_by_server);
                assert_eq!(abort.invoke_id, 128);
                assert_eq!(abort.abort_reason, AbortReason::BUFFER_OVERFLOW);
            }
            other => panic!("expected capacity Abort, got {other:?}"),
        }

        // Alternate retransmission and gap traffic every second. The window-one
        // baseline produces a NAK for each, providing a dispatch barrier and
        // proving that every original session remains live without new saves.
        for second in 1..=15 {
            sleep_until(started + Duration::from_secs(second)).await;
            for invoke_id in 0..128 {
                let seq = if second % 2 == 0 { 0 } else { 2 };
                send_segment_with_window(&client, invoke_id, seq, 1, true, &[0xEE]).await;
                match recv_apdu(&mut rx, "stalled session activity").await {
                    Apdu::SegmentAck(ack) => {
                        assert!(ack.negative_ack && ack.sent_by_server);
                        assert_eq!(ack.invoke_id, invoke_id);
                        assert_eq!(ack.sequence_number, 0);
                    }
                    other => panic!("session expired during activity setup: {other:?}"),
                }
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(16),
            "setup too slow"
        );
        sleep_until(started + Duration::from_secs(17)).await;

        // Protocol activity is only two seconds old. Under the old activity-only
        // policy all 128 slots are still occupied and this gets BUFFER_OVERFLOW.
        // No manually pruned map or test clock participates in this dispatch.
        send_segment_with_window(&client, 128, 0, 1, true, &admitted[0]).await;
        expect_positive_ack(&mut rx, 128, 0).await;
        assert_eq!(present_value(&server).await, initial_value);

        // Cleanup itself emits nothing. A current noninitial segment still gets
        // the existing invalid-state Abort, rather than resurrecting old data.
        send_segment_with_window(&client, 0, 1, 1, false, &discarded[1]).await;
        match recv_apdu(&mut rx, "noninitial after cleanup").await {
            Apdu::Abort(abort) => {
                assert!(abort.sent_by_server);
                assert_eq!(abort.invoke_id, 0);
                assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
            }
            other => panic!("expected invalid-state Abort, got {other:?}"),
        }
        assert_eq!(present_value(&server).await, initial_value);

        send_segment_with_window(&client, 128, 1, 1, false, &admitted[1]).await;
        expect_positive_ack(&mut rx, 128, 1).await;
        assert!(matches!(recv_apdu(&mut rx, "admitted write").await,
            Apdu::SimpleAck(ack) if ack.invoke_id == 128));
        assert_eq!(present_value(&server).await, "admitted-after-cleanup");

        // Same-key sequence zero can open a new incarnation: no tombstone or
        // fairness claim. Only its new payload may reach the service.
        let reopened = split_into(&write_property_payload("new-incarnation"), 2);
        send_segment_with_window(&client, 0, 0, 1, true, &reopened[0]).await;
        expect_positive_ack(&mut rx, 0, 0).await;
        send_segment_with_window(&client, 0, 1, 1, false, &reopened[1]).await;
        expect_positive_ack(&mut rx, 0, 1).await;
        assert!(matches!(recv_apdu(&mut rx, "reopened write").await,
            Apdu::SimpleAck(ack) if ack.invoke_id == 0));
        assert_eq!(present_value(&server).await, "new-incarnation");
        assert!(timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err());
    })
    .await
    .expect("bounded hostile repetition test exceeded 25 seconds");
}

#[tokio::test]
async fn request_progress_new_in_order_saves_sustain_transfer_beyond_16_seconds() {
    timeout(Duration::from_secs(25), async {
        let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
        let text = "progress-is-not-total-age";
        let chunks = split_into(&write_property_payload(text), 3);
        send_segment_with_window(&client, 42, 0, 1, true, &chunks[0]).await;
        expect_positive_ack(&mut rx, 42, 0).await;
        let started = TokioInstant::now();
        let mut last_seq = 0;
        for second in 1..=18 {
            sleep_until(started + Duration::from_secs(second)).await;
            if second == 9 || second == 18 {
                last_seq += 1;
                send_segment_with_window(
                    &client,
                    42,
                    last_seq,
                    1,
                    second != 18,
                    &chunks[last_seq as usize],
                )
                .await;
                expect_positive_ack(&mut rx, 42, last_seq).await;
            } else {
                // Retransmissions preserve protocol activity but cannot replace
                // the saved bytes or be the reason the progress budget resets.
                send_segment_with_window(&client, 42, last_seq, 1, true, &[0xEE]).await;
                assert!(matches!(recv_apdu(&mut rx, "retransmission NAK").await,
                    Apdu::SegmentAck(ack) if ack.negative_ack && ack.sent_by_server
                        && ack.invoke_id == 42 && ack.sequence_number == last_seq));
            }
        }
        assert!(started.elapsed() >= Duration::from_secs(18));
        assert!(
            matches!(recv_apdu(&mut rx, "progressing write completes").await,
            Apdu::SimpleAck(ack) if ack.invoke_id == 42)
        );
        assert_eq!(present_value(&server).await, text);
    })
    .await
    .expect("bounded progressing transfer exceeded 25 seconds");
}

#[tokio::test]
async fn request_progress_failed_initial_or_next_save_leaves_no_incarnation() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let initial_value = present_value(&server).await;
    let chunks = split_into(&write_property_payload("only-successful-saves"), 2);
    for failed_seq in [0, 1] {
        if failed_seq == 1 {
            send_segment_with_window(&client, 43, 0, 1, true, &chunks[0]).await;
            expect_positive_ack(&mut rx, 43, 0).await;
        }
        send_segment_with_window(&client, 43, failed_seq, 1, true, &[0; 1477]).await;
        assert!(matches!(recv_apdu(&mut rx, "unsaveable segment").await,
            Apdu::Abort(abort) if abort.sent_by_server && abort.invoke_id == 43
                && abort.abort_reason == AbortReason::BUFFER_OVERFLOW));
        send_segment_with_window(&client, 43, 1, 1, false, &chunks[1]).await;
        assert!(
            matches!(recv_apdu(&mut rx, "failed save cannot sustain state").await,
            Apdu::Abort(abort) if abort.sent_by_server && abort.invoke_id == 43
                && abort.abort_reason == AbortReason::INVALID_APDU_IN_THIS_STATE)
        );
        assert_eq!(present_value(&server).await, initial_value);
    }
    send_segment_with_window(&client, 43, 0, 1, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 43, 0).await;
    send_segment_with_window(&client, 43, 1, 1, false, &chunks[1]).await;
    expect_positive_ack(&mut rx, 43, 1).await;
    assert!(
        matches!(recv_apdu(&mut rx, "new successful incarnation").await,
        Apdu::SimpleAck(ack) if ack.invoke_id == 43)
    );
    assert_eq!(present_value(&server).await, "only-successful-saves");
}
