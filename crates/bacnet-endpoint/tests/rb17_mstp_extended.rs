//! RB-17 MS/TP endpoint with extended frames (#1663): the opt-in lifts the
//! APDU bound an identity may advertise from 480 to 1476, carries NPDUs of
//! up to 1497 octets, and leaves the default as it was. The RB-17 flows run
//! with the option on in `rb17_mstp_proof.rs` and `rb17_mstp_flows.rs`.
//!
//! Evidence level: simulator (LoopbackSerial), NOT physical bench or
//! on-wire conformance.

#[path = "support/mstp_wire.rs"]
mod mstp_wire;
#[path = "support/rb17_harness.rs"]
mod rb17_harness;

use std::time::Duration;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::mstp::codec::{MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA};
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpExecutionMode, MstpTransport};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::{Bytes, BytesMut};
use mstp_wire::{FrameType, MstpFrame};
use rb17_harness::*;

/// An NPDU of `length` octets; `expecting_reply` sets the control octet's
/// data-expecting-reply bit.
fn npdu(length: usize, expecting_reply: bool, tag: u8) -> Vec<u8> {
    let mut npdu: Vec<u8> = (0..length)
        .map(|index| (index as u8).wrapping_mul(29) ^ tag)
        .collect();
    npdu[0] = 0x01;
    npdu[1] = if expecting_reply { 0x04 } else { 0x00 };
    npdu
}

/// Without the option the endpoint is as before: standard frames, and an
/// identity advertising 1476 is refused. With it, the transport carries
/// APDUs of 1476 and the same identity builds.
#[test]
fn the_option_lifts_the_identity_bound_to_1476() {
    let (serial, _peer) = LoopbackSerial::pair();
    let standard = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .build_transport()
        .unwrap();
    assert!(!standard.config().extended_frames);
    assert_eq!(standard.max_npdu_length(), MAX_STANDARD_MPDU_DATA);
    assert_eq!(standard.local_receive_apdu_capacity(), 480);

    let (serial, _peer) = LoopbackSerial::pair();
    let error = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .identity(identity(Frames::Extended))
        .build_session()
        .err()
        .expect("a 1476 identity on standard frames is refused");
    assert!(
        error.to_string().contains("bound 480 (standard frames)"),
        "{error}"
    );

    let (serial, _peer) = LoopbackSerial::pair();
    let extended = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .extended_frames(true)
        .build_transport()
        .unwrap();
    assert!(extended.config().extended_frames);
    assert_eq!(extended.max_npdu_length(), MAX_EXTENDED_MPDU_DATA);
    assert_eq!(extended.local_receive_apdu_capacity(), 1476);
    assert_eq!(extended.egress_apdu_limit(), 1476);

    let (serial, _peer) = LoopbackSerial::pair();
    let id = identity(Frames::Extended);
    assert_eq!(id.max_apdu_length(), 1476);
    assert!(MstpEndpointBuilder::new(serial, SESSION_MAC)
        .extended_frames(true)
        .database(db_with_analog(&id, 1, 1.0))
        .identity(id)
        .build_session()
        .is_ok());
}

/// The endpoint's transport, with the option, and a peer with extended
/// frames pass 1497-octet NPDUs both ways: a request and its answer in the
/// reply window, then one that expects no reply (Clauses 9.3.9, 9.3.10).
/// Without the option the endpoint refuses an NPDU over 501 at the send.
#[tokio::test(start_paused = true)]
async fn a_1497_octet_npdu_passes_with_the_option() {
    let (endpoint_end, peer_end) = LoopbackSerial::pair();
    let mut endpoint = MstpEndpointBuilder::new(endpoint_end, 0)
        .max_master(1)
        .baud_rate(38_400)
        .extended_frames(true)
        .build_transport()
        .unwrap();
    let mut peer = MstpTransport::new(
        peer_end,
        MstpConfig {
            this_station: 1,
            max_master: 1,
            baud_rate: 38_400,
            extended_frames: true,
            ..MstpConfig::default()
        },
    );
    let mut at_endpoint = endpoint.start().await.unwrap();
    let mut at_peer = peer.start().await.unwrap();
    let wait = Duration::from_secs(10);

    let request = npdu(MAX_EXTENDED_MPDU_DATA, true, 0x3C);
    let answer = npdu(MAX_EXTENDED_MPDU_DATA, false, 0xA5);
    endpoint.send_unicast(&request, &[1]).await.unwrap();
    let received = tokio::time::timeout(wait, at_peer.recv())
        .await
        .expect("the request arrives")
        .unwrap();
    assert_eq!(received.npdu.as_ref(), request.as_slice());
    received
        .reply_tx
        .expect("a request carries a reply channel")
        .send(Bytes::from(answer.clone()))
        .unwrap();
    let replied = tokio::time::timeout(wait, at_endpoint.recv())
        .await
        .expect("the answer arrives")
        .unwrap();
    assert_eq!(replied.npdu.as_ref(), answer.as_slice());

    let back = npdu(MAX_EXTENDED_MPDU_DATA, false, 0x0F);
    peer.send_unicast(&back, &[0]).await.unwrap();
    let delivered = tokio::time::timeout(wait, at_endpoint.recv())
        .await
        .expect("the NPDU arrives")
        .unwrap();
    assert_eq!(delivered.npdu.as_ref(), back.as_slice());
    let counts = endpoint.diagnostics().snapshot();
    assert_eq!(counts.mac.parser_errors, 0);
    assert_eq!(counts.wait_for_reply_timeouts, 0);
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();

    let (serial, _peer) = LoopbackSerial::pair();
    let mut standard = MstpEndpointBuilder::new(serial, 0)
        .build_transport()
        .unwrap();
    let _npdus = standard.start().await.unwrap();
    let refused = standard
        .send_unicast(&npdu(MAX_EXTENDED_MPDU_DATA, false, 1), &[1])
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, Error::Encoding(text) if text.contains("standard-frame maximum 501")),
        "{refused}"
    );
    standard.stop().await.unwrap();
}

/// The longest Object_Name the 1476-octet ComplexAck carries here, with
/// room for its headers.
const LONG_NAME: usize = 1_400;

/// A session with the option and a 1476 identity answers a ReadProperty
/// whose ComplexAck is far over 480 octets, from a requester that accepts
/// 1476, promptly and in an extended frame (Type 33).
async fn long_answer_impl(mode: MstpExecutionMode) {
    let name: String = (0..LONG_NAME)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect();
    let id = identity(Frames::Extended);
    let db = db_with_named_analog(&id, 1, &name, 1.0);
    let mut h = Harness::with_database(mode, SessionRole::Both, Frames::Extended, id, db).await;
    let request = read_apdu(5, 1, PropertyIdentifier::OBJECT_NAME, 1476);
    h.peer_send(&MstpFrame {
        frame_type: FrameType::BACNET_DATA_EXPECTING_REPLY,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: local_npdu(request, true),
    })
    .await;
    let reply = h
        .wait_for("the long ComplexAck", |f| {
            f.destination == PEER_MAC
                && matches!(
                    f.frame_type,
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                        | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
                )
        })
        .await;
    assert_eq!(
        reply.frame_type,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    );
    let npdu = decode_npdu(reply.data.clone()).unwrap();
    let apdu_length = npdu.payload.len();
    assert!(apdu_length > 480 && apdu_length <= 1476, "{apdu_length}");
    assert!(reply.data.len() > MAX_STANDARD_MPDU_DATA);
    let (invoke_id, ack) = decode_complex_ack(&reply);
    assert_eq!(invoke_id, 5);
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    let mut expected = BytesMut::new();
    encode_property_value(&mut expected, &PropertyValue::CharacterString(name)).unwrap();
    assert_eq!(ack.property_value, expected.to_vec());
    assert!(decode_apdu(npdu.payload).is_ok());
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_extended_long_answer_tokio() {
    long_answer_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_extended_long_answer_dedicated() {
    long_answer_impl(MstpExecutionMode::DedicatedThread).await;
}
