//! The full server a session builds (#2617, part 3a), over an in-memory
//! link with broadcasts on paused time: the read-only server a session
//! will build when it is given no builder of its own, and a request that
//! is on the link before the session starts.
use super::*;
use crate::session::eventually::eventually;
use bacnet_client::client::{BACnetClient, Broadcast};
use bacnet_encoding::apdu::{ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::who_has::WhoHasObject;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, EngineeringUnits, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;
use std::time::Duration;

use crate::session::wire::{self, Wire};

const NODE: u8 = 1;
const PEER: u8 = 2;
const NODE_DEVICE: u32 = 1001;
const WAIT: Duration = Duration::from_secs(5);

fn av7() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap()
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// The node's database: its Device, AV-7 and AI-1.
fn database() -> ObjectDatabase {
    let identity = crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let units = EngineeringUnits::DEGREES_CELSIUS;
    let value = AnalogValueObject::new(7, "AV-7", units).unwrap();
    let input = AnalogInputObject::new(1, "AI-1", units).unwrap();
    crate::build_database_with_extra(&identity, vec![Box::new(value), Box::new(input)]).unwrap()
}

/// A session of both roles on node [`NODE`] of `wire`, with Device
/// [`NODE_DEVICE`], the [`database`] and a full server from `builder`, not
/// started, and its send log.
fn session(
    wire: &Wire,
    builder: ServerBuilder<TestTransport>,
) -> (EndpointSession<TestTransport>, SendLog) {
    let port = wire.port(NODE);
    let sent = port.sent();
    let session = EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
        .with_database(database())
        .with_server(builder);
    (session, sent)
}

/// A standalone client on node [`PEER`].
async fn peer(wire: &Wire) -> BACnetClient<TestTransport> {
    BACnetClient::generic_builder()
        .transport(wire.port(PEER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap()
}

/// Whether `result` is the server's denial of a covered mutation.
fn denied<T: std::fmt::Debug>(result: Result<T, Error>) -> bool {
    match result {
        Err(error) => {
            error.remote()
                == Some(RemoteError::Error {
                    class: ErrorClass::SERVICES,
                    code: ErrorCode::SERVICE_REQUEST_DENIED,
                })
        }
        Ok(value) => panic!("allowed: {value:?}"),
    }
}

/// AV-7's Present_Value as `peer` reads it from the node.
async fn present_value(peer: &BACnetClient<TestTransport>) -> f32 {
    let ack = peer
        .read_property(
            &Wire::mac(NODE),
            av7(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();
    f32::from_be_bytes(ack.property_value[1..5].try_into().unwrap())
}

/// The read-only server: a peer reads, discovers the node and its objects
/// and subscribes to COV, which the server reports; no peer writes,
/// creates or deletes, each refused SERVICES / SERVICE_REQUEST_DENIED with
/// nothing changed.
#[tokio::test(start_paused = true)]
async fn the_read_only_server_answers_reads_discovery_and_subscriptions_only() {
    let wire = Wire::new();
    let (mut session, _) = session(&wire, read_only_server());
    session.start().await.unwrap();
    let peer = peer(&wire).await;
    let node = Wire::mac(NODE);
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let before = present_value(&peer).await;

    // An application-tagged REAL, 5.0.
    let mut real = vec![0x44];
    real.extend_from_slice(&5.0f32.to_be_bytes());
    let pv = PropertyIdentifier::PRESENT_VALUE;
    assert!(denied(
        peer.write_property(&node, av7(), pv, None, real.clone(), Some(8))
            .await
    ));
    // A character string, "x".
    let text = vec![0x72, 0x00, b'x'];
    let description = PropertyIdentifier::DESCRIPTION;
    assert!(denied(
        peer.write_property(&node, device, description, None, text, None)
            .await
    ));
    let created = peer
        .create_object(
            &node,
            ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            vec![],
        )
        .await;
    assert!(denied(created));
    assert!(denied(peer.delete_object(&node, av7()).await));
    assert_eq!(present_value(&peer).await, before, "nothing changed");
    let server = session.server().unwrap();
    assert!(
        server.read_local(&av7(), pv, None).await.is_ok(),
        "still there"
    );

    // Discovery: an I-Am for the node, an I-Have for its AV-7.
    let mut i_have = peer.i_have_events();
    peer.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the node's I-Am", async || {
        peer.discovered_devices()
            .await
            .iter()
            .any(|found| found.object_identifier == device && found.mac_address == node)
    })
    .await;
    let object = WhoHasObject::Identifier(av7());
    peer.who_has(Broadcast::Global, object, None).await.unwrap();
    let have = tokio::time::timeout(WAIT, i_have.recv())
        .await
        .expect("the node's I-Have")
        .unwrap();
    assert_eq!(have.i_have.object_identifier, av7());

    // A COV subscription is accepted and reported.
    let mut heard = peer.cov_notifications();
    peer.subscribe_cov(&node, 3, ai1(), false, Some(600))
        .await
        .unwrap();
    tokio::time::timeout(WAIT, heard.recv())
        .await
        .expect("the first notification")
        .unwrap();
    server
        .set_present_value_local(&ai1(), PropertyValue::Real(9.0))
        .await
        .unwrap();
    let changed = tokio::time::timeout(WAIT, heard.recv())
        .await
        .expect("a notification of the change")
        .unwrap();
    assert_eq!(changed.notification.monitored_object_identifier, ai1());

    let mut peer = peer;
    peer.stop().await.unwrap();
    session.stop().await.unwrap();
}

/// A ReadProperty of AV-7's Present_Value from [`PEER`] under `invoke_id`,
/// as the link hands it to the node.
fn read_request(invoke_id: u8) -> bacnet_transport::port::ReceivedNpdu {
    let mut parameters = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: av7(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut parameters);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: parameters.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire::frame(&Wire::mac(PEER), npdu.freeze(), false)
}

/// The APDUs the node sent [`PEER`].
fn answers(sent: &SendLog) -> Vec<Apdu> {
    let peer: MacAddr = Wire::mac(PEER);
    sent.frames()
        .into_iter()
        .filter(|frame| frame.mac == peer)
        .filter_map(|frame| decode_apdu(decode_npdu(frame.npdu).ok()?.payload).ok())
        .collect()
}

/// A request already on the link when the session starts reaches the
/// server: the session takes the server's seat as soon as its link is up,
/// before it builds the server, so the request waits in the seat's queue
/// and is answered once the start is done, never counted as finding no
/// server.
#[tokio::test(start_paused = true)]
async fn a_request_on_the_link_before_start_is_the_servers() {
    let wire = Wire::new();
    let (mut session, sent) = session(&wire, BACnetServer::generic_builder());
    wire.deliver(&Wire::mac(NODE), read_request(41));
    session.start().await.unwrap();
    eventually(WAIT, "the answer", async || !answers(&sent).is_empty()).await;
    let answers = answers(&sent);
    assert!(
        matches!(&answers[..], [Apdu::ComplexAck(ack)] if ack.invoke_id == 41),
        "{answers:?}"
    );
    assert_eq!(session.policy_counters().await.no_server_role, 0);
    assert_eq!(session.server_seat_counters().apdus, 1);
    session.stop().await.unwrap();
}
