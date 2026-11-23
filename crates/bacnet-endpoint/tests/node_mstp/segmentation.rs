//! The node segments on MS/TP (#2617, part 3c): a node of both roles with
//! the identity's default segmentation, against raw-frame peers that take
//! their turn when the node passes station 7 the token.
//!
//! Station 7 is the token partner and a device the node's client reads and
//! writes; station 8 is a requester of the node's server, whose frames go
//! out on station 7's turn (the node can't tell). A peer answers a Data
//! Expecting Reply frame at once, inside the node's reply window, and sends
//! anything else on its next turn.
use std::cell::Cell;
use std::collections::VecDeque;

use super::*;
use bacnet_encoding::apdu::{ComplexAck, SegmentAck, SimpleAck};
use bacnet_encoding::primitives::encode_app_character_string;
use bacnet_endpoint::identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::rpm::ReadPropertyMultipleRequest;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::EngineeringUnits;

const OTHER_MAC: u8 = 8;

/// A name about twice the link's largest APDU, which the server sends a
/// 480-octet requester in several segments.
fn long_name(frames: Frames) -> String {
    "segmented-".repeat(usize::from(frames.max_apdu()) / 5)
}

/// A node of both roles with the identity's default segmentation, both
/// ways, its AI-1 at 11.0 and its AI-2 named [`long_name`].
async fn segmenting_node(mode: MstpExecutionMode, frames: Frames) -> Harness {
    let id = DeviceIdentity::new(2001, 42)
        .unwrap()
        .with_max_apdu(frames.max_apdu())
        .unwrap();
    assert_eq!(id.segmentation(), Segmentation::SEGMENTED_BOTH);
    let mut ai1 = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    ai1.set_present_value(11.0);
    let ai2 = AnalogInputObject::new(2, long_name(frames), EngineeringUnits::NO_UNITS).unwrap();
    let db = build_database_with_extra(&id, vec![Box::new(ai1), Box::new(ai2)]).unwrap();
    Box::pin(Harness::with_database(
        mode,
        SessionRole::Both,
        frames,
        id,
        db,
    ))
    .await
}

/// A data frame from `source` to the node.
fn from(source: u8, frame_type: FrameType, apdu: &Apdu) -> MstpFrame {
    MstpFrame {
        frame_type,
        destination: SESSION_MAC,
        source,
        data: local_npdu(
            encoded(apdu),
            frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY,
        ),
    }
}

/// Station 7's answer to the node's read of its AI-2 name: `name`, in
/// segments of at most `segment` octets.
fn answer_segments(invoke_id: u8, name: &str, segment: usize) -> Vec<Apdu> {
    let mut value = BytesMut::new();
    encode_app_character_string(&mut value, name).unwrap();
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut ack)
    .unwrap();
    let ack = ack.freeze();
    let parts: Vec<Bytes> = ack.chunks(segment).map(Bytes::copy_from_slice).collect();
    let count = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(seq, part)| {
            Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: seq + 1 < count,
                invoke_id,
                sequence_number: Some(seq as u8),
                proposed_window_size: Some(1),
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: part,
            })
        })
        .collect()
}

/// A positive Segment-ACK for `seq`, from the requester (`server` false) or
/// the answering side.
fn segment_ack(server: bool, invoke_id: u8, seq: u8) -> Apdu {
    Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: server,
        invoke_id,
        sequence_number: seq,
        actual_window_size: 1,
    })
}

/// What one peer does with an APDU the node sent it: the frame it sends
/// back, if any. `reply` says the node's frame expects a reply, which then
/// goes at once; otherwise it waits for the peer's turn.
type React<'a> = dyn FnMut(u8, Apdu, bool) -> Option<MstpFrame> + 'a;

/// What the token ring showed while the peers ran.
#[derive(Default, Debug)]
struct Ring {
    /// Reply Postponed frames, to station 7 and to station 8.
    postponed: [usize; 2],
    /// Data frames the node sent, by kind: (expecting a reply, the APDU).
    sent: Vec<(u8, bool, Apdu)>,
}

/// Run the ring: pass the token back and forth, answer polls, send the
/// peers' frames on station 7's turn, and hand each APDU the node sends a
/// peer to `react`, until `done`. The node's frames must all reach their
/// peers within 2,000 of its frames.
async fn run_ring(
    h: &mut Harness,
    mut pending: VecDeque<MstpFrame>,
    react: &mut React<'_>,
    mut done: impl FnMut() -> bool,
) -> Ring {
    let mut ring = Ring::default();
    let mut awaiting_reply = false;
    let mut holds_token = false;
    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
    for _ in 0..2_000 {
        if done() && !awaiting_reply && !holds_token {
            return ring;
        }
        let frame = h.wait_for("the node's next frame", |_| true).await;
        let peer = frame.destination;
        let mut turn = false;
        match frame.frame_type {
            FrameType::TOKEN if peer == PEER_MAC => {
                holds_token = true;
                turn = true;
            }
            FrameType::POLL_FOR_MASTER if peer == PEER_MAC => {
                h.peer_send(&reply_to_poll(SESSION_MAC, PEER_MAC)).await;
            }
            FrameType::REPLY_POSTPONED if peer == PEER_MAC || peer == OTHER_MAC => {
                ring.postponed[usize::from(peer == OTHER_MAC)] += 1;
                awaiting_reply = false;
                turn = holds_token;
            }
            FrameType::BACNET_DATA_EXPECTING_REPLY | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                if peer == PEER_MAC || peer == OTHER_MAC =>
            {
                let expects = frame.frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY;
                let apdu = apdu(&frame).expect("an APDU");
                ring.sent.push((peer, expects, apdu.clone()));
                let back = react(peer, apdu, expects);
                if expects {
                    let back = back.expect("the peer answers a frame awaiting its reply");
                    h.peer_send(&back).await;
                } else {
                    pending.extend(back);
                    if awaiting_reply {
                        // The node's answer to the peer's last frame.
                        awaiting_reply = false;
                        turn = holds_token;
                    }
                }
            }
            _ => {}
        }
        // The peer's turn: its next frame, then the token back once it
        // waits for nothing.
        while turn {
            match pending.pop_front() {
                Some(next) => {
                    h.peer_send(&next).await;
                    if next.frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY {
                        awaiting_reply = true;
                        turn = false;
                    }
                }
                None => {
                    holds_token = false;
                    turn = false;
                    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
                }
            }
        }
    }
    panic!("the ring did not finish: {ring:?}; {}", h.link_report());
}

/// The node's client reads station 7's AI-2 name, which comes back in
/// segments, while its server answers station 8's read of its own AI-2
/// name in segments. Both finish, each Segment-ACK coming from the role
/// whose transfer it is.
///
/// What the MAC does with a Segment-ACK is pinned here: station 7 sends
/// each segment after the first as a frame expecting a reply, and the
/// client's Segment-ACK is an ordinary send, not the MAC's reply, so the
/// node answers each with Reply Postponed and sends the Segment-ACK when it
/// next holds the token. That costs a token round per segment, no more:
/// the transfer completes, well inside the segment timeout.
async fn segmented_answers_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = segmenting_node(mode, frames).await;
    let client = Arc::new(h.session.client().unwrap());
    // The raw peer writes standard frames only: its segments fit 480
    // octets whatever the node's link takes.
    let segment = 472;
    let name_7 = "station-7-".repeat(120);
    let reader = Arc::clone(&client);
    let read = tokio::spawn(async move {
        reader
            .read_property(
                &[PEER_MAC][..],
                oid(ObjectType::ANALOG_INPUT, 2),
                PropertyIdentifier::OBJECT_NAME,
                None,
            )
            .await
    });
    // Station 8 reads the node's AI-2 name, taking segmented answers.
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut request);
    let other_read = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(16),
        max_apdu_length: 480,
        invoke_id: 61,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: request.freeze(),
    });
    let pending = VecDeque::from([from(
        OTHER_MAC,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        &other_read,
    )]);

    let mut answers_7: Vec<Apdu> = Vec::new();
    let client_done = Cell::new(false);
    let mut server_payload = BytesMut::new();
    let server_done = Cell::new(false);
    let mut react = |peer: u8, apdu: Apdu, _reply: bool| -> Option<MstpFrame> {
        match (peer, apdu) {
            (PEER_MAC, Apdu::ConfirmedRequest(asked)) => {
                answers_7 = answer_segments(asked.invoke_id, &name_7, segment);
                assert!(answers_7.len() >= 2, "station 7's answer needs segments");
                Some(from(
                    PEER_MAC,
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    &answers_7[0],
                ))
            }
            (PEER_MAC, Apdu::SegmentAck(ack)) => {
                assert!(!ack.sent_by_server && !ack.negative_ack, "{ack:?}");
                let next = usize::from(ack.sequence_number) + 1;
                if next < answers_7.len() {
                    Some(from(
                        PEER_MAC,
                        FrameType::BACNET_DATA_EXPECTING_REPLY,
                        &answers_7[next],
                    ))
                } else {
                    client_done.set(true);
                    None
                }
            }
            (OTHER_MAC, Apdu::ComplexAck(answer)) => {
                assert!(answer.segmented, "the server's answer needs segments");
                assert_eq!(answer.invoke_id, 61);
                server_payload.extend_from_slice(&answer.service_ack);
                server_done.set(!answer.more_follows);
                let seq = answer.sequence_number.unwrap();
                Some(from(
                    OTHER_MAC,
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    &segment_ack(false, 61, seq),
                ))
            }
            (peer, other) => panic!("station {peer} got {other:?}"),
        }
    };
    let ring = run_ring(&mut h, pending, &mut react, || {
        client_done.get() && server_done.get()
    })
    .await;
    let name = tokio::time::timeout(WAIT, read)
        .await
        .expect("the client's read ended")
        .unwrap()
        .expect("the client's read");
    assert!(name.property_value.ends_with(name_7.as_bytes()));
    let answer = ReadPropertyACK::decode(&server_payload).unwrap();
    assert!(
        answer
            .property_value
            .ends_with(long_name(frames).as_bytes())
    );
    // Each of station 7's segments after the first expected a reply: the
    // client's Segment-ACK went as an ordinary send after Reply Postponed.
    assert_eq!(ring.postponed[0], answers_7.len() - 1, "{ring:?}");
    let client_acks = ring
        .sent
        .iter()
        .filter(|(peer, expects, apdu)| {
            *peer == PEER_MAC
                && !expects
                && matches!(apdu, Apdu::SegmentAck(ack) if !ack.sent_by_server)
        })
        .count();
    assert_eq!(client_acks, answers_7.len());
    assert!(
        !ring
            .sent
            .iter()
            .any(|(_, _, apdu)| matches!(apdu, Apdu::Abort(_))),
        "{ring:?}"
    );
    drop(client);
    h.shutdown().await;
}

/// The node's client writes a Description too long for station 7's APDU,
/// which goes in segments, and station 7 sends the node's server a
/// ReadPropertyMultiple in segments: each is acknowledged by the side that
/// answers it and completes.
async fn segmented_requests_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = segmenting_node(mode, frames).await;
    let client = Arc::new(h.session.client().unwrap());
    // Station 7's I-Am: a device that takes 128 octets and segments.
    let mut i_am = BytesMut::new();
    IAmRequest {
        object_identifier: oid(ObjectType::DEVICE, 7007),
        max_apdu_length: 128,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        vendor_id: 42,
    }
    .encode(&mut i_am);
    let i_am = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: i_am.freeze(),
    });
    h.peer_deliver(&MstpFrame {
        frame_type: FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        destination: 0xFF,
        source: PEER_MAC,
        data: local_npdu(encoded(&i_am), false),
    })
    .await;
    tokio::time::timeout(WAIT, async {
        while client.discovered_devices().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the node's client heard station 7's I-Am");

    let mut description = BytesMut::new();
    encode_app_character_string(&mut description, &"described-".repeat(30)).unwrap();
    let writer = Arc::clone(&client);
    let write = tokio::spawn(async move {
        writer
            .write_property(
                &[PEER_MAC][..],
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::DESCRIPTION,
                None,
                description.to_vec(),
                None,
            )
            .await
    });
    // Station 7's ReadPropertyMultiple of the node's AI-1, 30 times, in two
    // segments.
    let mut rpm = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                };
                30
            ],
        }],
    }
    .encode(&mut rpm)
    .unwrap();
    let rpm = rpm.freeze();
    let half = rpm.len() / 2;
    let rpm_segment = |seq: u8, part: Bytes| {
        from(
            PEER_MAC,
            FrameType::BACNET_DATA_EXPECTING_REPLY,
            &Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: true,
                more_follows: seq == 0,
                segmented_response_accepted: true,
                max_segments: Some(16),
                max_apdu_length: 480,
                invoke_id: 71,
                sequence_number: Some(seq),
                proposed_window_size: Some(1),
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                service_request: part,
            }),
        )
    };
    let second = rpm_segment(1, rpm.slice(half..));
    let pending = VecDeque::from([rpm_segment(0, rpm.slice(..half))]);

    let mut second = Some(second);
    let mut write_segments = 0usize;
    let write_done = Cell::new(false);
    let rpm_done = Cell::new(false);
    let mut react = |peer: u8, apdu: Apdu, _reply: bool| -> Option<MstpFrame> {
        assert_eq!(peer, PEER_MAC);
        match apdu {
            // The server acknowledges station 7's segments: the first
            // brings the second; the last it may acknowledge before it
            // answers.
            Apdu::SegmentAck(ack) if ack.sent_by_server && ack.invoke_id == 71 => {
                assert!(!ack.negative_ack, "{ack:?}");
                match ack.sequence_number {
                    0 => second.take(),
                    1 => None,
                    other => panic!("an acknowledgement of segment {other}"),
                }
            }
            Apdu::ComplexAck(answer) if answer.invoke_id == 71 => {
                assert_eq!(
                    answer.service_choice,
                    ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
                );
                rpm_done.set(true);
                None
            }
            // The client's write, segment by segment.
            Apdu::ConfirmedRequest(segment) => {
                assert!(segment.segmented, "the write needs segments: {segment:?}");
                assert_eq!(
                    segment.service_choice,
                    ConfirmedServiceChoice::WRITE_PROPERTY
                );
                write_segments += 1;
                let seq = segment.sequence_number.unwrap();
                let back = if segment.more_follows {
                    segment_ack(true, segment.invoke_id, seq)
                } else {
                    write_done.set(true);
                    Apdu::SimpleAck(SimpleAck {
                        invoke_id: segment.invoke_id,
                        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                    })
                };
                Some(from(
                    PEER_MAC,
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    &back,
                ))
            }
            other => panic!("station 7 got {other:?}"),
        }
    };
    let ring = run_ring(&mut h, pending, &mut react, || {
        write_done.get() && rpm_done.get()
    })
    .await;
    tokio::time::timeout(WAIT, write)
        .await
        .expect("the client's write ended")
        .unwrap()
        .expect("the client's write");
    assert!(write_segments >= 3, "{ring:?}");
    assert!(
        !ring
            .sent
            .iter()
            .any(|(_, _, apdu)| matches!(apdu, Apdu::Abort(_))),
        "{ring:?}"
    );
    drop(client);
    h.shutdown().await;
}

variants!(segmented_answers_impl: segmented_answers_tokio, segmented_answers_dedicated, segmented_answers_extended_tokio, segmented_answers_extended_dedicated);
variants!(segmented_requests_impl: segmented_requests_tokio, segmented_requests_dedicated, segmented_requests_extended_tokio, segmented_requests_extended_dedicated);

/// Station 7 sends the node's client a ConfirmedCOVNotification in
/// segments of a window of one, each as a frame expecting a reply (#2646).
/// The client's Segment-ACK for each, server bit set, is the MAC's reply:
/// none is postponed. The Simple-ACK for the whole notification goes on the
/// node's next turn, and the client publishes the notification once.
async fn segmented_notification_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = segmenting_node(mode, frames).await;
    let client = h.session.client().unwrap();
    let mut events = client.cov_notifications();
    let mut body = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 61,
        initiating_device_identifier: oid(ObjectType::DEVICE, 7007),
        monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        time_remaining: 120,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut body)
    .unwrap();
    let body = body.freeze();
    let size = body.len().div_ceil(3);
    let mut segments: VecDeque<MstpFrame> = (0..3)
        .map(|seq| {
            from(
                PEER_MAC,
                FrameType::BACNET_DATA_EXPECTING_REPLY,
                &Apdu::ConfirmedRequest(ConfirmedRequest {
                    segmented: true,
                    more_follows: seq < 2,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length: 480,
                    invoke_id: 72,
                    sequence_number: Some(seq as u8),
                    proposed_window_size: Some(1),
                    service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    service_request: body.slice(seq * size..body.len().min((seq + 1) * size)),
                }),
            )
        })
        .collect();
    let pending = VecDeque::from([segments.pop_front().unwrap()]);
    let mut acked = Vec::new();
    let done = Cell::new(false);
    let mut react = |peer: u8, apdu: Apdu, _reply: bool| -> Option<MstpFrame> {
        assert_eq!(peer, PEER_MAC);
        match apdu {
            Apdu::SegmentAck(ack) if ack.invoke_id == 72 => {
                assert!(ack.sent_by_server && !ack.negative_ack, "{ack:?}");
                acked.push(ack.sequence_number);
                segments.pop_front()
            }
            Apdu::SimpleAck(ack) if ack.invoke_id == 72 => {
                assert_eq!(
                    ack.service_choice,
                    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
                );
                done.set(true);
                None
            }
            other => panic!("station 7 got {other:?}"),
        }
    };
    let ring = run_ring(&mut h, pending, &mut react, || done.get()).await;
    assert_eq!(acked, [0, 1, 2], "{ring:?}");
    assert_eq!(
        ring.postponed[0], 0,
        "each Segment-ACK is the reply: {ring:?}"
    );
    let event = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("the notification is published")
        .unwrap();
    assert_eq!(event.subscriber_process_identifier(), 61);
    assert!(events.try_recv().is_err(), "published once");
    drop(client);
    h.shutdown().await;
}

variants!(segmented_notification_impl: segmented_notification_tokio, segmented_notification_dedicated, segmented_notification_extended_tokio, segmented_notification_extended_dedicated);
