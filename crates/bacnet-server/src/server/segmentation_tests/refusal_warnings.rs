//! A peer's flood of APDUs that don't decode, or of segments the server
//! refuses or receives out of order, warns once per interval for each
//! throttle, with the occurrences since the last warning; the rest log at
//! debug (#1950).

use super::*;
use crate::server::warn_spy::WarnSpy;
use bacnet_encoding::apdu::AbortPdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::warn_throttle::WARNING_INTERVAL;
use bacnet_types::enums::Segmentation;
use request_reassembly::{recv_apdu, send_apdu, send_segment_with_window, start_reassembly_server};

const UNDECODABLE: &str = "Server failed to decode received APDU";
const WINDOW: &str = "Rejecting segmented request with invalid proposed window size";
const GAP: &str = "Segment gap detected, sending negative SegmentAck";
const NO_SEGMENT_0: &str = "Received non-initial segment without prior segment 0, aborting";

type Rx = mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>;

async fn expect_abort(rx: &mut Rx, invoke_id: u8) {
    let Apdu::Abort(abort) = recv_apdu(rx, "refused segment").await else {
        panic!("expected an Abort");
    };
    assert_eq!(abort.invoke_id, invoke_id);
}

/// Send `count` APDUs of PDU type 15, which doesn't exist; a segment 0
/// proposing window size 0; a segment 2 after segment 0, a gap; and then
/// `count` segments numbered 1 with no segment 0 before them. Each refusal
/// is answered, and those answers show everything before them was handled.
/// Invoke IDs start at `base`.
async fn flood(client: &LoopbackTransport, rx: &mut Rx, count: u8, base: u8) {
    for _ in 0..count {
        let npdu = [0x01, 0x00, 0xF0];
        client.send_unicast(&npdu, &[0x02]).await.unwrap();
    }
    // The encoder refuses window size 0, so this one is written by hand: a
    // segmented ConfirmedRequest with more to follow, segment 0, window 0,
    // WriteProperty.
    let window_0 = [0x01, 0x00, 0x0C, 0x05, base, 0, 0, 15, 0];
    client.send_unicast(&window_0, &[0x02]).await.unwrap();
    expect_abort(rx, base).await;

    let gapped = base + 1;
    send_segment_with_window(client, gapped, 0, 1, true, &[0]).await;
    let Apdu::SegmentAck(ack) = recv_apdu(rx, "segment 0").await else {
        panic!("expected a SegmentAck");
    };
    assert!(!ack.negative_ack);
    send_segment_with_window(client, gapped, 2, 1, true, &[0]).await;
    let Apdu::SegmentAck(ack) = recv_apdu(rx, "segment 2").await else {
        panic!("expected a SegmentAck");
    };
    assert!(ack.negative_ack);
    // End the session, so it doesn't expire and answer later.
    let abort = AbortPdu {
        sent_by_server: false,
        invoke_id: gapped,
        abort_reason: AbortReason::OTHER,
    };
    send_apdu(client, &Apdu::Abort(abort)).await;

    for invoke_id in base + 2..base + 2 + count {
        send_segment_with_window(client, invoke_id, 1, 1, true, &[0]).await;
        expect_abort(rx, invoke_id).await;
    }
}

/// The warnings with one of `messages`, and the occurrences each reported.
fn warned(spy: &WarnSpy, messages: &[&str]) -> Vec<(String, Option<u64>)> {
    spy.warnings()
        .into_iter()
        .filter(|warning| messages.contains(&warning.message.as_str()))
        .map(|warning| (warning.message, warning.occurrences))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_flood_of_bad_apdus_and_refused_segments_warns_once_per_interval() {
    let (spy, _guard) = WarnSpy::install();
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let segment = [WINDOW, GAP, NO_SEGMENT_0];
    flood(&client, &mut rx, 5, 0).await;
    assert_eq!(
        warned(&spy, &[UNDECODABLE]),
        [(UNDECODABLE.into(), Some(1))]
    );
    assert_eq!(warned(&spy, &segment), [(WINDOW.into(), Some(1))]);

    tokio::time::advance(WARNING_INTERVAL).await;
    flood(&client, &mut rx, 3, 100).await;
    assert_eq!(
        warned(&spy, &[UNDECODABLE]),
        [(UNDECODABLE.into(), Some(1)), (UNDECODABLE.into(), Some(5))],
        "four held back, and the first of the next burst"
    );
    assert_eq!(
        warned(&spy, &segment),
        [(WINDOW.into(), Some(1)), (WINDOW.into(), Some(7))],
        "the gap and five refusals held back, and the first of the next burst"
    );
    // Nothing else the floods set off warned (the other is the startup
    // warning about the default vendor identifier).
    assert_eq!(spy.warnings().len(), 5, "{:?}", spy.warnings());
    drop(server);
}
