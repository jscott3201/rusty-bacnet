//! The node on MS/TP (#2617, part 3a): an endpoint session of both roles
//! that builds a full server at start (`with_server`) beside a full
//! client, over simulated serial, against the raw-frame peer of
//! `support/rb17_harness.rs`. Each scenario runs on the Tokio and the
//! dedicated-thread MAC, with standard and with extended frames, on paused
//! time.
//!
//! What it shows, at simulator level (not a bench or on-wire conformance):
//! the server answers inside the reply window through the node's MAC, or
//! after Reply Postponed once it is free, with the request's identity; a
//! confirmed COV notification for the node is its client's, answered while
//! the server is busy; the server's confirmed notification and the
//! client's request draw on the one pool of invoke IDs, and each answer
//! completes the role that asked; and a Who-Is now draws the node's I-Am,
//! which the narrow responder never sent.

#[path = "support/mstp_wire.rs"]
mod mstp_wire;
#[path = "support/rb17_harness.rs"]
mod rb17_harness;

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{
    Apdu, ConfirmedRequest, SimpleAck, UnconfirmedRequest, decode_apdu, encode_apdu,
};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint::session::SessionRole;
use bacnet_server::server::BACnetServer;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::mstp::MstpExecutionMode;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice,
};
use bytes::{Bytes, BytesMut};
use mstp_wire::{FrameType, MstpFrame, reply_to_poll, token_frame};
use rb17_harness::*;

/// A node of both roles whose session builds a full server, with the
/// server's own defaults: nothing here writes.
async fn node(mode: MstpExecutionMode, frames: Frames) -> Harness {
    Box::pin(Harness::with_server(
        mode,
        SessionRole::Both,
        frames,
        BACnetServer::generic_builder(),
    ))
    .await
}

/// The APDU a data frame carries, if it carries one.
fn apdu(frame: &MstpFrame) -> Option<Apdu> {
    let npdu = decode_npdu(frame.data.clone()).ok()?;
    if npdu.message_type.is_some() {
        return None;
    }
    decode_apdu(npdu.payload).ok()
}

/// `apdu` encoded.
fn encoded(apdu: &Apdu) -> Bytes {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, apdu).unwrap();
    bytes.freeze()
}

/// `service` with `parameters` under `invoke_id`, from a requester that
/// takes APDUs of up to 480 octets.
fn confirmed(invoke_id: u8, service: ConfirmedServiceChoice, parameters: BytesMut) -> Bytes {
    encoded(&Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: parameters.freeze(),
    }))
}

/// A frame from the peer to the node carrying `apdu`.
fn to_node(frame_type: FrameType, apdu: Bytes, expecting_reply: bool) -> MstpFrame {
    MstpFrame {
        frame_type,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: local_npdu(apdu, expecting_reply),
    }
}

fn is_answer_to_peer(frame: &MstpFrame) -> bool {
    frame.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY && frame.destination == PEER_MAC
}

/// The node's answer to the peer's last request: inside the reply window,
/// or at a token opportunity after Reply Postponed.
async fn answer(h: &mut Harness, what: &str) -> Apdu {
    let frame = h
        .wait_for(what, |frame| {
            is_answer_to_peer(frame)
                || (frame.frame_type == FrameType::REPLY_POSTPONED && frame.destination == PEER_MAC)
        })
        .await;
    let frame = if frame.frame_type == FrameType::REPLY_POSTPONED {
        h.drive_token_until_data(what, is_answer_to_peer).await
    } else {
        frame
    };
    apdu(&frame).unwrap_or_else(|| panic!("{what}: an APDU"))
}

/// Let `wait` of paused time pass, then the data frames the node sent the
/// peer meanwhile.
async fn data_to_peer_within(h: &mut Harness, wait: Duration) -> Vec<Apdu> {
    tokio::time::sleep(wait).await;
    let mut found = Vec::new();
    while let Ok(frame) = h.writes.try_recv() {
        assert_eq!(frame.source, SESSION_MAC);
        if frame.destination == PEER_MAC
            && let Some(apdu) = apdu(&frame)
        {
            found.push(apdu);
        }
    }
    found
}

/// (a) The server answers a ReadProperty inside the reply window, through
/// the node's MAC, from its database, with no Reply Postponed first.
async fn prompt_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = node(mode, frames).await;
    h.peer_deliver(&data_expecting(1, 9)).await;
    let mut postponed = false;
    let reply = h
        .wait_for("the prompt ComplexAck", |frame| {
            postponed |= frame.frame_type == FrameType::REPLY_POSTPONED;
            is_answer_to_peer(frame)
        })
        .await;
    assert!(!postponed, "the prompt answer had a Reply Postponed first");
    expect_present_value(&reply, 9, 11.0);
    assert_eq!(h.session.server_seat_counters().apdus, 1);
    assert_eq!(h.session.policy_counters().await.no_server_role, 0);
    h.shutdown().await;
}

/// A ConfirmedCOVNotification for the node's client under `invoke_id`.
fn cov_notification(invoke_id: u8) -> Bytes {
    let mut parameters = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 4,
        initiating_device_identifier: oid(ObjectType::DEVICE, 77),
        monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 3),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            // An application-tagged REAL, 42.0.
            value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut parameters)
    .unwrap();
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        parameters,
    )
}

/// (b) While the server waits for its database, a ReadProperty draws Reply
/// Postponed from the node's MAC, and (c) a ConfirmedCOVNotification in the
/// same window is the client's: it answers, and only its seat counts it.
/// Once the database is free the server's answer goes at a token
/// opportunity, once, with the request's invoke ID.
async fn postponed_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = node(mode, frames).await;
    let client = h.session.full_client().expect("the node's full client");
    let mut heard = client.cov_notifications();
    let db = Arc::clone(h.session.server().expect("the full server").database());
    let held = db.write().await;
    h.peer_deliver(&data_expecting(1, 33)).await;
    h.wait_for("Reply Postponed", |frame| {
        frame.frame_type == FrameType::REPLY_POSTPONED && frame.destination == PEER_MAC
    })
    .await;

    let (client_apdus, server_apdus) = (
        h.session.client_seat_counters().apdus,
        h.session.server_seat_counters().apdus,
    );
    let notification = to_node(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        cov_notification(17),
        true,
    );
    h.peer_deliver(&notification).await;
    let ack = answer(&mut h, "the client's SimpleAck").await;
    assert!(
        matches!(
            ack,
            Apdu::SimpleAck(SimpleAck {
                invoke_id: 17,
                service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            })
        ),
        "{ack:?}"
    );
    assert_eq!(h.session.client_seat_counters().apdus, client_apdus + 1);
    assert_eq!(h.session.server_seat_counters().apdus, server_apdus);
    let received = tokio::time::timeout(WAIT, heard.recv())
        .await
        .expect("the client heard the notification")
        .unwrap();
    assert_eq!(received.subscriber_process_identifier(), 4);

    drop(held);
    let late = h
        .drive_token_until_data("the server's late ComplexAck", is_answer_to_peer)
        .await;
    expect_present_value(&late, 33, 11.0);
    let more = data_to_peer_within(&mut h, Duration::from_secs(3)).await;
    assert!(
        !more
            .iter()
            .any(|apdu| matches!(apdu, Apdu::ComplexAck(ack) if ack.invoke_id == 33)),
        "answered twice: {more:?}"
    );
    assert!(heard.try_recv().is_err(), "heard once");
    drop(client);
    h.shutdown().await;
}

/// The peer's SubscribeCOV to the node's AI-1, confirmed, for an hour.
fn subscribe(invoke_id: u8) -> Bytes {
    let mut parameters = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 6,
        monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(3_600),
    }
    .encode(&mut parameters)
    .unwrap();
    confirmed(invoke_id, ConfirmedServiceChoice::SUBSCRIBE_COV, parameters)
}

/// A SimpleAck from the peer for `service` under `invoke_id`.
fn simple_ack(invoke_id: u8, service: ConfirmedServiceChoice) -> Bytes {
    encoded(&Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: service,
    }))
}

/// Pass the node the token and answer, inside its reply window, each
/// request it sends the peer, until `count` have come: what each was.
async fn answer_requests(h: &mut Harness, count: usize) -> Vec<ConfirmedRequest> {
    let mut answered = Vec::new();
    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
    for _ in 0..256 {
        let frame = h.wait_for("the node's requests", |_| true).await;
        if frame.frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY
            && frame.destination == PEER_MAC
        {
            let Some(Apdu::ConfirmedRequest(request)) = apdu(&frame) else {
                panic!("a confirmed request: {frame:?}");
            };
            let reply = match request.service_choice {
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION => {
                    local_npdu(simple_ack(request.invoke_id, request.service_choice), false)
                }
                ConfirmedServiceChoice::READ_PROPERTY => {
                    let read = ReadPropertyRequest::decode(&request.service_request).unwrap();
                    assert_eq!(read.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
                    complex_ack_npdu(request.invoke_id, 22.0)
                }
                other => panic!("an unexpected request: {other:?}"),
            };
            h.peer_send(&MstpFrame {
                frame_type: FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                destination: SESSION_MAC,
                source: PEER_MAC,
                data: reply,
            })
            .await;
            answered.push(request);
            if answered.len() == count {
                return answered;
            }
            continue;
        }
        match frame.frame_type {
            FrameType::POLL_FOR_MASTER if frame.destination == PEER_MAC => {
                h.peer_send(&reply_to_poll(SESSION_MAC, PEER_MAC)).await;
            }
            FrameType::TOKEN if frame.destination == PEER_MAC => {
                h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
            }
            _ => {}
        }
    }
    panic!("the node sent {} of {count} requests", answered.len());
}

/// (d) The server's confirmed COV notification to the peer's subscription
/// and the full client's read of the peer go out together, under different
/// invoke IDs from the node's one pool, and each answer completes the role
/// that asked: the read returns the peer's value, and the notification
/// goes no second time.
async fn one_pool_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = node(mode, frames).await;
    let client = Arc::new(h.session.full_client().expect("the node's full client"));
    h.peer_deliver(&to_node(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        subscribe(5),
        true,
    ))
    .await;
    let ack = answer(&mut h, "the subscription's SimpleAck").await;
    assert!(
        matches!(&ack, Apdu::SimpleAck(ack) if ack.invoke_id == 5),
        "{ack:?}"
    );
    let reader = Arc::clone(&client);
    let read = tokio::spawn(async move {
        reader
            .read_property(
                &[PEER_MAC][..],
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let requests = answer_requests(&mut h, 2).await;
    let services: Vec<_> = requests
        .iter()
        .map(|request| request.service_choice)
        .collect();
    assert!(
        services.contains(&ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION)
            && services.contains(&ConfirmedServiceChoice::READ_PROPERTY),
        "{services:?}"
    );
    assert_ne!(requests[0].invoke_id, requests[1].invoke_id);
    let value = tokio::time::timeout(WAIT, read)
        .await
        .expect("the read ended")
        .unwrap()
        .expect("the client's answer");
    assert_eq!(value.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    // Past the node's APDU timeout: the notification was acknowledged.
    let more = data_to_peer_within(&mut h, Duration::from_secs(5)).await;
    assert!(
        !more.iter().any(|apdu| matches!(
            apdu,
            Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        )),
        "the notification went again: {more:?}"
    );
    assert_eq!(h.session.active_leases(), 0);
    assert_eq!(h.session.policy_counters().await.unclaimed_terminal, 0);
    drop(client);
    h.shutdown().await;
}

/// (e) A Who-Is draws the node's I-Am: the link's largest APDU, the
/// identity's segmentation and vendor.
async fn who_is_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = node(mode, frames).await;
    let mut parameters = BytesMut::new();
    WhoIsRequest::all().encode(&mut parameters);
    let who_is = encoded(&Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: parameters.freeze(),
    }));
    h.peer_deliver(&MstpFrame {
        frame_type: FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        destination: 0xFF,
        source: PEER_MAC,
        data: local_npdu(who_is, false),
    })
    .await;
    let i_am = h
        .drive_token_until_data("the node's I-Am", |frame| {
            frame.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                && matches!(
                    apdu(frame),
                    Some(Apdu::UnconfirmedRequest(request))
                        if request.service_choice == UnconfirmedServiceChoice::I_AM
                )
        })
        .await;
    let Some(Apdu::UnconfirmedRequest(request)) = apdu(&i_am) else {
        unreachable!()
    };
    let i_am = IAmRequest::decode(&request.service_request).unwrap();
    assert_eq!(i_am.object_identifier, oid(ObjectType::DEVICE, 2001));
    assert_eq!(i_am.max_apdu_length, u32::from(frames.max_apdu()));
    assert_eq!(i_am.segmentation_supported, Segmentation::NO_SEGMENTATION);
    assert_eq!(i_am.vendor_id, 42);
    h.shutdown().await;
}

macro_rules! variants {
    ($scenario:ident: $tokio:ident, $dedicated:ident, $extended_tokio:ident, $extended_dedicated:ident) => {
        #[tokio::test(start_paused = true)]
        async fn $tokio() {
            Box::pin($scenario(MstpExecutionMode::Tokio, Frames::Standard)).await;
        }

        #[tokio::test(start_paused = true)]
        async fn $dedicated() {
            Box::pin($scenario(dedicated(), Frames::Standard)).await;
        }

        #[tokio::test(start_paused = true)]
        async fn $extended_tokio() {
            Box::pin($scenario(MstpExecutionMode::Tokio, Frames::Extended)).await;
        }

        #[tokio::test(start_paused = true)]
        async fn $extended_dedicated() {
            Box::pin($scenario(dedicated(), Frames::Extended)).await;
        }
    };
}

variants!(prompt_impl: node_prompt_tokio, node_prompt_dedicated, node_prompt_extended_tokio, node_prompt_extended_dedicated);
variants!(postponed_impl: node_postponed_tokio, node_postponed_dedicated, node_postponed_extended_tokio, node_postponed_extended_dedicated);
variants!(one_pool_impl: node_one_pool_tokio, node_one_pool_dedicated, node_one_pool_extended_tokio, node_one_pool_extended_dedicated);
variants!(who_is_impl: node_who_is_tokio, node_who_is_dedicated, node_who_is_extended_tokio, node_who_is_extended_dedicated);
