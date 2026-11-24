//! The client seat's tiers (#2635): copies of a burst of I-Ams fill at most
//! half of it, so a client that falls behind while they arrive, here one
//! that never reads, still gets its COV notification and the answer to its
//! own request; the burst's overflow is dropped and counted. Nor does such a
//! client hold up the server's answer to a segmented event notification,
//! whose whole request the server hands it, whatever room its copies take
//! (#2743).
use super::*;
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_endpoint_core::coordinator::{CanonicalPeer, LeaseMetadata, TerminalPolicy};
use bacnet_endpoint_core::seat::MAX_REASSEMBLED_COPY_OCTETS;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::test_transport::SendLog;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::{Bytes, BytesMut};
use std::time::Duration;

use super::super::eventually::eventually;

use crate::session::server_seat::tests::wire::{self, Wire};

const NODE: u8 = 1;
const PEER: u8 = 2;
const BURST: usize = 300;

/// Device `instance`'s I-Am, from a station of its own.
fn i_am(instance: u32) -> bacnet_transport::port::ReceivedNpdu {
    let mut npdu = vec![0x01, 0x00, 0x10, 0x00, 0xC4];
    npdu.extend_from_slice(&((8u32 << 22) | instance).to_be_bytes());
    npdu.extend_from_slice(&[0x22, 0x01, 0xE0, 0x91, 0x03, 0x21, 0x2A]);
    let station = encode_bip_mac([10, 0, (instance >> 8) as u8, instance as u8], 47_808);
    wire::frame(&station, Bytes::from(npdu), true)
}

#[tokio::test(start_paused = true)]
async fn a_seat_nobody_reads_keeps_room_for_the_answer_to_its_clients_request() {
    let wire = Wire::new();
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let transport = wire.port_with_queue(NODE, 1_024).build();
    let mut session = EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(crate::DeviceIdentity::new(123, 42).unwrap());
    session.start().await.unwrap();
    let mut seat = session.client_seat().unwrap();
    let lease = seat
        .coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&peer),
            ConfirmedServiceChoice::WRITE_PROPERTY,
            TerminalPolicy::SimpleAck,
        ))
        .unwrap();
    let answer = Bytes::from(vec![0x01, 0x00, 0x20, lease.invoke_id(), 0x0f]);

    wire.hold(&node);
    for instance in 0..BURST as u32 {
        wire.deliver(&node, i_am(instance));
    }
    // A ConfirmedCOVNotification, which beside a server role is the
    // client's alone.
    let notification = Bytes::from_static(&[0x01, 0x04, 0x00, 0x05, 0x22, 0x01]);
    wire.deliver(&node, wire::frame(&peer, notification, false));
    wire.deliver(&node, wire::frame(&peer, answer, false));
    wire.release(&node);
    eventually(
        Duration::from_secs(5),
        "the whole burst classified",
        async || {
            let counters = session.client_seat_counters();
            counters.apdus + counters.full_drops == BURST as u64 + 2
        },
    )
    .await;

    // Half of the seat's 256 slots took I-Ams; the notification and the
    // answer took slots of the tiers kept for them.
    let counters = session.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (130, 172));
    let queued: Vec<ReceivedApdu> = std::iter::from_fn(|| seat.apdus.try_recv().ok()).collect();
    assert_eq!(queued.len(), 130);
    assert!(
        queued[..128]
            .iter()
            .all(|received| received.apdu[0] == 0x10)
    );
    assert_eq!(queued[128].apdu.as_ref(), [0x00, 0x05, 0x22, 0x01]);
    assert_eq!(queued[129].apdu.as_ref(), [0x20, lease.invoke_id(), 0x0f]);
    assert!(session.is_running());

    drop(seat);
    session.stop().await.unwrap();
}

/// A ConfirmedEventNotification's parameters, from Device 7.
fn event_parameters() -> Bytes {
    event_parameters_with("too hot".into())
}

/// [`event_parameters`] with `text` for its message.
fn event_parameters_with(text: String) -> Bytes {
    use bacnet_services::alarm_event::EventNotificationRequest;
    use bacnet_types::enums::{EventState, EventType, NotifyType, ObjectType};
    use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some(text),
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    };
    let mut parameters = BytesMut::new();
    bacnet_encoding::constructed::encode_event_notification(&request, &mut parameters).unwrap();
    parameters.freeze()
}

/// `parameters` of an event notification under `invoke_id`, in segments
/// of `size` octets proposing `window`, each in an NPDU that expects a
/// reply.
fn event_segments(invoke_id: u8, parameters: &Bytes, size: usize, window: u8) -> Vec<Bytes> {
    let chunks: Vec<Bytes> = (0..parameters.len().div_ceil(size))
        .map(|n| parameters.slice(n * size..parameters.len().min((n + 1) * size)))
        .collect();
    let last = chunks.len() - 1;
    chunks
        .into_iter()
        .enumerate()
        .map(|(seq, chunk)| {
            let segment = Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: true,
                more_follows: seq < last,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id,
                sequence_number: Some(seq as u8),
                proposed_window_size: Some(window),
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                service_request: chunk,
            });
            let mut payload = BytesMut::new();
            encode_apdu(&mut payload, &segment).unwrap();
            let npdu = Npdu {
                expecting_reply: true,
                payload: payload.freeze(),
                ..Npdu::default()
            };
            let mut encoded = BytesMut::new();
            encode_npdu(&mut encoded, &npdu).unwrap();
            encoded.freeze()
        })
        .collect()
}

/// Whether the node has sent an APDU that `want` takes.
fn has_sent(log: &SendLog, want: impl Fn(&Apdu) -> bool) -> bool {
    log.frames()
        .into_iter()
        .filter_map(|frame| decode_apdu(decode_npdu(frame.npdu).ok()?.payload).ok())
        .any(|apdu| want(&apdu))
}

/// Send [`event_parameters`] under `invoke_id` from the peer to the node in
/// 3 segments, each once the node has acknowledged the one before, and wait
/// for the server's Simple-ACK.
async fn notify(wire: &Wire, log: &SendLog, invoke_id: u8) {
    let parameters = event_parameters();
    let size = parameters.len().div_ceil(3);
    notify_with(wire, log, invoke_id, &parameters, size, 1).await;
}

/// [`notify`], with `parameters` in segments of `size` octets, a `window`
/// of them at a time.
async fn notify_with(
    wire: &Wire,
    log: &SendLog,
    invoke_id: u8,
    parameters: &Bytes,
    size: usize,
    window: u8,
) {
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let segments = event_segments(invoke_id, parameters, size, window);
    let (last, mut acknowledged) = (segments.len() - 1, 0);
    for (seq, segment) in segments.into_iter().enumerate() {
        wire.deliver(&node, wire::frame(&peer, segment, false));
        if seq != 0 && seq != acknowledged + usize::from(window) && seq != last {
            continue;
        }
        let ack = |apdu: &Apdu| {
            matches!(apdu, Apdu::SegmentAck(ack)
                if ack.invoke_id == invoke_id && usize::from(ack.sequence_number) == seq)
        };
        eventually(WAIT, "the segment acknowledged", async || {
            has_sent(log, ack)
        })
        .await;
        acknowledged = seq;
    }
    eventually(WAIT, "the server's Simple-ACK", async || {
        has_sent(
            log,
            |apdu| matches!(apdu, Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id),
        )
    })
    .await;
}

const WAIT: Duration = Duration::from_secs(5);

#[tokio::test(start_paused = true)]
async fn a_full_seat_holds_up_no_answer_to_a_segmented_event_notification() {
    let wire = Wire::new();
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let port = wire.port_with_queue(NODE, 1_024);
    let log = port.sent();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(crate::DeviceIdentity::new(123, 42).unwrap());
    session.start().await.unwrap();
    let mut seat = session.client_seat().unwrap();

    // Confirmed COV notifications nobody reads fill the seat down to the
    // last quarter that confirmed requests leave.
    let notification = Bytes::from_static(&[0x01, 0x04, 0x00, 0x05, 0x22, 0x01]);
    for _ in 0..192 {
        wire.deliver(&node, wire::frame(&peer, notification.clone(), false));
    }
    eventually(WAIT, "the seat filled", async || {
        session.client_seat_counters().apdus == 192
    })
    .await;

    // The server acknowledges each segment and answers; the whole
    // notification it hands on finds no room, and is dropped and counted.
    notify(&wire, &log, 9).await;
    let counters = session.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (192, 1));
    let queued: Vec<ReceivedApdu> = std::iter::from_fn(|| seat.apdus.try_recv().ok()).collect();
    assert_eq!(queued.len(), 192);
    assert!(queued.iter().all(|received| received.apdu[3] == 0x01));

    // With room again the next one reaches the seat whole, once, and none
    // of its segments does.
    notify(&wire, &log, 10).await;
    let copy = tokio::time::timeout(WAIT, seat.apdus.recv())
        .await
        .unwrap()
        .unwrap();
    let Ok(Apdu::ConfirmedRequest(whole)) = decode_apdu(copy.apdu.clone()) else {
        panic!("not a request: {:?}", copy.apdu);
    };
    assert!(!whole.segmented && whole.invoke_id == 10);
    assert_eq!(whole.service_request, event_parameters());
    assert!(copy.reply_tx.is_none() && copy.direct_response.is_none());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(seat.apdus.try_recv().is_err(), "once, and no segment");
    assert_eq!(session.client_seat_counters().full_drops, 1);

    drop(seat);
    session.stop().await.unwrap();
}

/// A client seat nobody reads holds at most
/// [`MAX_REASSEMBLED_COPY_OCTETS`] in copies of reassembled requests
/// (#2743): past it the server still answers, and the copy is dropped and
/// counted; once the client has taken and let go of its copies, the next
/// one fits.
#[tokio::test(start_paused = true)]
async fn copies_past_their_octets_are_dropped_and_the_server_still_answers() {
    let wire = Wire::new();
    let port = wire.port_with_queue(NODE, 1_024);
    let log = port.sent();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(crate::DeviceIdentity::new(123, 42).unwrap());
    session.start().await.unwrap();
    let mut seat = session.client_seat().unwrap();
    // About as long as a request in segments gets: 255 segments of 1476
    // octets, 127 at a time. So many of their copies fit in the bound, and
    // one more doesn't.
    let large = event_parameters_with("x".repeat(376_000));
    let fits = MAX_REASSEMBLED_COPY_OCTETS / (large.len() + 4);
    assert!(fits < 192, "the bound, not the seat's slots, refuses it");
    for invoke_id in 11..=11 + fits as u8 {
        notify_with(&wire, &log, invoke_id, &large, 1476, 127).await;
    }
    let counters = session.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (fits as u64, 1));

    let taken: Vec<ReceivedApdu> = std::iter::from_fn(|| seat.apdus.try_recv().ok()).collect();
    assert_eq!(taken.len(), fits);
    drop(taken);
    notify_with(&wire, &log, 40, &large, 1476, 127).await;
    let copy = tokio::time::timeout(WAIT, seat.apdus.recv())
        .await
        .unwrap()
        .unwrap();
    let Ok(Apdu::ConfirmedRequest(whole)) = decode_apdu(copy.apdu.clone()) else {
        panic!("not a request");
    };
    assert_eq!((whole.invoke_id, whole.service_request), (40, large));
    let counters = session.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (fits as u64 + 1, 1));

    drop(seat);
    session.stop().await.unwrap();
}
