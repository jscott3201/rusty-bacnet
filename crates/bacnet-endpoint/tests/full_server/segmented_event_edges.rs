//! The edges of a segmented ConfirmedEventNotification to a node with both
//! roles (#2743): a routed source, a single segment, a node that receives no
//! segments, a transfer sent again, a client made again, and event values
//! the client can't read. Each is answered and published as one that comes
//! whole would be.
use super::segmentation::{identity, node};
use super::segmentation_wire::{abort, count, from_peer, peer, sent};
use super::segmented_events::{
    EVENT, acknowledged, both, deliver, event_segments, heard, nothing_more, notification,
    parameters, refusals, segments, send, send_from, whole,
};
use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::{AbortReason, Segmentation};

/// Segment-ACKs the node sent under `invoke_id`.
fn segment_acks(log: &SendLog, invoke_id: u8) -> usize {
    count(
        log,
        |apdu| matches!(apdu, Apdu::SegmentAck(a) if a.invoke_id == invoke_id),
    )
}

/// The NPDU destination of every answer the node sent the peer.
fn answer_destinations(log: &SendLog) -> Vec<Option<NpduAddress>> {
    log.frames()
        .into_iter()
        .filter(|frame| frame.mac == peer())
        .filter_map(|frame| {
            let npdu = decode_npdu(frame.npdu).ok()?;
            let apdu = decode_apdu(npdu.payload.clone()).ok()?;
            matches!(apdu, Apdu::SimpleAck(_) | Apdu::SegmentAck(_)).then_some(npdu.destination)
        })
        .collect()
}

/// From a device behind a router: the answers go back through the router
/// to it, and the client hears the router's address and the device's, as
/// it does of one that comes whole.
#[tokio::test(start_paused = true)]
async fn a_routed_notification_keeps_its_source() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    let behind = NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[0x0a, 0x0b]),
    };

    deliver(&wire, &whole(0xb1, 61), Some(&behind));
    sent(&log, "the whole one's answer", acknowledged(0xb1)).await;
    send_from(&wire, &log, &event_segments(0xb2, 62, 3, 1), Some(&behind)).await;
    sent(&log, "the segmented one's answer", acknowledged(0xb2)).await;
    for process in [61, 62] {
        let event = heard(&mut events).await;
        assert_eq!(event.notification, notification(process));
        assert_eq!(event.source_mac, peer());
        assert_eq!(event.source_network, Some(7));
        assert_eq!(event.source_address, Some(behind.mac_address.clone()));
    }
    nothing_more(&mut events).await;
    // Two Simple-ACKs and three Segment-ACKs, each to the device behind.
    let destinations = answer_destinations(&log);
    assert_eq!(destinations.len(), 5);
    assert!(destinations.iter().all(|to| *to == Some(behind.clone())));
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// One segment that is also the last: one Segment-ACK, one Simple-ACK, one
/// notification.
#[tokio::test(start_paused = true)]
async fn a_single_segment_is_one_notification() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();

    send(&wire, &log, &event_segments(0xc1, 71, 1, 1)).await;
    sent(&log, "the Simple-ACK", acknowledged(0xc1)).await;
    assert_eq!(heard(&mut events).await.notification, notification(71));
    nothing_more(&mut events).await;
    assert_eq!(segment_acks(&log, 0xc1), 1);
    assert_eq!(count(&log, acknowledged(0xc1)), 1);
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// A node whose Device receives no segments: the server aborts the first
/// segment, once, and nothing is published or acknowledged.
#[tokio::test(start_paused = true)]
async fn a_node_that_receives_no_segments_aborts_once_and_publishes_nothing() {
    let wire = Wire::new();
    let mode = Segmentation::NO_SEGMENTATION;
    let identity = identity(NODE_DEVICE, mode);
    let (mut session, log, _link) = node(&wire, SessionRole::Both, identity).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();

    from_peer(&wire, &event_segments(0xd1, 81, 3, 1)[0]);
    let Apdu::Abort(aborted) = sent(&log, "the server's Abort", abort(true, 0xd1)).await else {
        unreachable!()
    };
    assert_eq!(
        aborted.abort_reason,
        AbortReason::SEGMENTATION_NOT_SUPPORTED
    );
    nothing_more(&mut events).await;
    assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::Abort(_))), 1);
    assert_eq!(refusals(&log), 1);
    assert_eq!(segment_acks(&log, 0xd1), 0);
    assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::SimpleAck(_))), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// The whole transfer sent again after its answer is a new transmission:
/// answered again and published again, as a whole one sent again is. The
/// client publishes once per transmission.
#[tokio::test(start_paused = true)]
async fn a_transfer_sent_again_after_its_answer_is_published_again() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    let parts = event_segments(0xe1, 91, 3, 1);

    for transmission in 1..=2 {
        send(&wire, &log, &parts).await;
        eventually(WAIT, "the answer", async || {
            count(&log, acknowledged(0xe1)) == transmission
        })
        .await;
        assert_eq!(heard(&mut events).await.notification, notification(91));
        nothing_more(&mut events).await;
    }
    // A whole one sent twice: the same.
    for transmission in 1..=2 {
        from_peer(&wire, &whole(0xe2, 92));
        eventually(WAIT, "the answer", async || {
            count(&log, acknowledged(0xe2)) == transmission
        })
        .await;
        assert_eq!(heard(&mut events).await.notification, notification(92));
        nothing_more(&mut events).await;
    }
    assert_eq!(segment_acks(&log, 0xe1), 6);
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// The node's client dropped and another made: the next notification
/// reaches the new one.
#[tokio::test(start_paused = true)]
async fn a_client_made_again_hears_the_next_notification() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let first = session.client().unwrap();
    let mut first_events = first.event_notifications();
    send(&wire, &log, &event_segments(0xf1, 101, 3, 1)).await;
    sent(&log, "the first answer", acknowledged(0xf1)).await;
    assert_eq!(
        heard(&mut first_events).await.notification,
        notification(101)
    );
    drop(first_events);
    drop(first);

    let second = session.client().unwrap();
    let mut events = second.event_notifications();
    send(&wire, &log, &event_segments(0xf2, 102, 3, 1)).await;
    sent(&log, "the second answer", acknowledged(0xf2)).await;
    assert_eq!(heard(&mut events).await.notification, notification(102));
    nothing_more(&mut events).await;
    assert_eq!(refusals(&log), 0);

    drop(second);
    session.stop().await.unwrap();
}

/// Event values the server takes as framed but the client can't read: the
/// server acknowledges the notification and the client publishes nothing,
/// whole or in segments alike.
#[tokio::test(start_paused = true)]
async fn event_values_the_client_cannot_read_are_acknowledged_not_published() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    // An out-of-range notification's values with nothing inside them.
    let mut values = parameters(111).to_vec();
    values.extend_from_slice(&[0xce, 0x5e, 0x5f, 0xcf]);
    let values = Bytes::from(values);

    let mut whole_one = whole(0xa1, 111);
    if let Apdu::ConfirmedRequest(request) = &mut whole_one {
        request.service_request = values.clone();
    }
    from_peer(&wire, &whole_one);
    sent(&log, "the whole one's answer", acknowledged(0xa1)).await;
    send(&wire, &log, &segments(EVENT, 0xa2, &values, 3, 1)).await;
    sent(&log, "the segmented one's answer", acknowledged(0xa2)).await;
    nothing_more(&mut events).await;
    assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::SimpleAck(_))), 2);
    assert_eq!(segment_acks(&log, 0xa2), 3);
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}
