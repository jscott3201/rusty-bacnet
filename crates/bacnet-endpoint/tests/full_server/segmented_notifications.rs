//! The node's client reassembles a segmented COV notification it answers
//! (#2646): on a node of both roles beside the server's own transfers, and
//! on a node with no server role, as far as the Device's Segmentation says.
use super::segmentation::{identity, node};
use super::segmentation_wire::{
    abort, ack, answer, count, from_peer, peer, read_segment, request_segment, sent,
};
use super::*;
use bacnet_client::client::COVEvent;
use bacnet_encoding::apdu::AbortPdu;
use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
};
use bacnet_types::enums::{AbortReason, Segmentation};

const COV_MULTIPLE: ConfirmedServiceChoice =
    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;

/// A COV-multiple from the peer for `process`, about AI 1 to AI 8.
pub(super) fn notification(process: u32) -> COVNotificationMultipleRequest {
    COVNotificationMultipleRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, PEER_DEVICE)
            .unwrap(),
        time_remaining: 300,
        timestamp: None,
        list_of_cov_notifications: (1..=8)
            .map(|instance| COVNotificationItem {
                monitored_object_identifier: ObjectIdentifier::new(
                    ObjectType::ANALOG_INPUT,
                    instance,
                )
                .unwrap(),
                list_of_values: vec![COVNotificationValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
                    time_of_change: None,
                }],
            })
            .collect(),
    }
}

/// `notification(process)` in three segments under `invoke_id`, each with
/// a window of one, as a conservative device sends them.
fn notification_segments(invoke_id: u8, process: u32) -> Vec<Apdu> {
    let mut body = BytesMut::new();
    notification(process).encode(&mut body).unwrap();
    let body = body.freeze();
    let size = body.len().div_ceil(3);
    (0..3)
        .map(|seq| {
            let data = body.slice(seq * size..body.len().min((seq + 1) * size));
            request_segment(invoke_id, seq as u8, seq < 2, COV_MULTIPLE, data)
        })
        .collect()
}

/// The node's Simple-ACK of a COV-multiple under `invoke_id`.
fn acknowledged(invoke_id: u8) -> impl Fn(&Apdu) -> bool {
    move |apdu| {
        matches!(apdu, Apdu::SimpleAck(ack)
            if ack.invoke_id == invoke_id && ack.service_choice == COV_MULTIPLE)
    }
}

/// The peer's Abort of its own request under `invoke_id`.
fn requester_abort(invoke_id: u8) -> Apdu {
    Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id,
        abort_reason: AbortReason::OTHER,
    })
}

async fn next_event(events: &mut tokio::sync::broadcast::Receiver<COVEvent>) -> COVEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("no notification published")
        .unwrap()
}

/// One peer sends the node's server a segmented ReadProperty and its
/// client a segmented COV-multiple under the same invoke ID: each role
/// acknowledges its own segments (both with the server bit set), the server
/// answers the read, and the client publishes and acknowledges the
/// notification once.
#[tokio::test(start_paused = true)]
async fn each_role_reassembles_its_own_request_under_one_invoke_id() {
    let wire = Wire::new();
    let (mut session, log, _link) = node(
        &wire,
        SessionRole::Both,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    let mut events = client.cov_notifications();
    let id = 0x42;
    let parts = notification_segments(id, 77);

    from_peer(&wire, &read_segment(id, 0));
    from_peer(&wire, &parts[0]);
    eventually(WAIT, "both roles' Segment-ACKs for segment 0", async || {
        count(&log, ack(true, id, 0)) == 2
    })
    .await;
    from_peer(&wire, &parts[1]);
    sent(&log, "the client's Segment-ACK for 1", ack(true, id, 1)).await;
    from_peer(&wire, &read_segment(id, 1));
    sent(&log, "the server's answer", answer(id)).await;
    from_peer(&wire, &parts[2]);
    sent(&log, "the client's Segment-ACK for 2", ack(true, id, 2)).await;
    sent(&log, "the client's Simple-ACK", acknowledged(id)).await;

    let COVEvent::Multiple(event) = next_event(&mut events).await else {
        panic!("expected the several-object form");
    };
    assert_eq!(event.notification, notification(77));
    assert_eq!(event.source_mac, peer());
    assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::Abort(_))), 0);
    assert_eq!(count(&log, acknowledged(id)), 1);
    assert!(events.try_recv().is_err(), "published once");

    drop(client);
    session.stop().await.unwrap();
}

/// The peer gives up its notification midway: the client's transfer ends
/// there, and the server's transfer with the same peer goes on to its
/// answer. A later segment of the notification finds nothing to join.
#[tokio::test(start_paused = true)]
async fn a_requesters_abort_ends_the_clients_transfer_only() {
    let wire = Wire::new();
    let (mut session, log, _link) = node(
        &wire,
        SessionRole::Both,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    let mut events = client.cov_notifications();
    let parts = notification_segments(0x43, 78);

    from_peer(&wire, &parts[0]);
    sent(&log, "the client's Segment-ACK", ack(true, 0x43, 0)).await;
    from_peer(&wire, &read_segment(0x44, 0));
    sent(&log, "the server's Segment-ACK", ack(true, 0x44, 0)).await;
    from_peer(&wire, &requester_abort(0x43));
    from_peer(&wire, &read_segment(0x44, 1));
    sent(&log, "the server's answer", answer(0x44)).await;
    from_peer(&wire, &parts[1]);
    let Apdu::Abort(aborted) = sent(&log, "the client's Abort", abort(true, 0x43)).await else {
        unreachable!()
    };
    assert_eq!(
        aborted.abort_reason,
        AbortReason::INVALID_APDU_IN_THIS_STATE
    );
    assert!(events.try_recv().is_err(), "nothing published");

    drop(client);
    session.stop().await.unwrap();
}

/// A node with no server role: its client answers every confirmed request
/// as a standalone one does, segmented notifications included, and a
/// requester's Abort reaches it.
#[tokio::test(start_paused = true)]
async fn a_client_only_node_reassembles_and_takes_a_requesters_abort() {
    let wire = Wire::new();
    let (mut session, log, _link) = node(
        &wire,
        SessionRole::ClientOnly,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    let mut events = client.cov_notifications();

    for (seq, part) in notification_segments(0x45, 79).iter().enumerate() {
        from_peer(&wire, part);
        sent(&log, "the client's Segment-ACK", ack(true, 0x45, seq as u8)).await;
    }
    sent(&log, "the client's Simple-ACK", acknowledged(0x45)).await;
    assert_eq!(
        next_event(&mut events)
            .await
            .subscriber_process_identifier(),
        79
    );

    let parts = notification_segments(0x46, 80);
    from_peer(&wire, &parts[0]);
    sent(&log, "the client's Segment-ACK", ack(true, 0x46, 0)).await;
    from_peer(&wire, &requester_abort(0x46));
    from_peer(&wire, &parts[1]);
    sent(&log, "the client's Abort", abort(true, 0x46)).await;
    assert!(events.try_recv().is_err());

    drop(client);
    session.stop().await.unwrap();
}

/// A Device that receives no segments: the client answers the first
/// segment with SEGMENTATION_NOT_SUPPORTED, once, as it always did.
#[tokio::test(start_paused = true)]
async fn a_node_that_receives_no_segments_still_aborts_the_first_segment() {
    for mode in [
        Segmentation::NO_SEGMENTATION,
        Segmentation::SEGMENTED_TRANSMIT,
    ] {
        let wire = Wire::new();
        let (mut session, log, _link) =
            node(&wire, SessionRole::Both, identity(NODE_DEVICE, mode)).await;
        let client = session.client().unwrap();
        from_peer(&wire, &notification_segments(0x47, 81)[0]);
        let Apdu::Abort(aborted) = sent(&log, "the client's Abort", abort(true, 0x47)).await else {
            unreachable!()
        };
        assert_eq!(
            aborted.abort_reason,
            AbortReason::SEGMENTATION_NOT_SUPPORTED,
            "{mode:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::Abort(_))), 1);
        assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::SegmentAck(_))), 0);

        drop(client);
        session.stop().await.unwrap();
    }
}

/// A stop midway through a notification ends its transfer: nothing more
/// goes out, not even once its timers have run out.
#[tokio::test(start_paused = true)]
async fn a_stop_midway_through_a_notification_sends_nothing_more() {
    let wire = Wire::new();
    let (mut session, log, _link) = node(
        &wire,
        SessionRole::Both,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    from_peer(&wire, &notification_segments(0x48, 82)[0]);
    sent(&log, "the client's Segment-ACK", ack(true, 0x48, 0)).await;
    let before = log.len();

    session.stop().await.unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(log.len(), before);
    drop(client);
}
