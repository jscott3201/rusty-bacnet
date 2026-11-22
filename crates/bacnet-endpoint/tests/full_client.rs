//! A full `BACnetClient` on an endpoint session's link (#2617), on paused
//! time over an in-memory link with broadcasts; `stop_waits` alone runs on
//! real time, and says why.
//!
//! The node under test is an endpoint session with its server role and
//! its client on the seat. Its peers are a
//! `bacnet-server` device with enough objects that reading its Object_List
//! takes a segmented answer, and, where a test needs one, a standalone
//! client or frames the test writes itself.
#![allow(dead_code)]

#[path = "support/eventually.rs"]
mod eventually;
#[path = "support/wire.rs"]
mod wire;

#[path = "full_client/bursts.rs"]
mod bursts;
#[path = "full_client/lifecycle.rs"]
mod lifecycle;
#[path = "full_client/numbers.rs"]
mod numbers;
#[path = "full_client/old_devices.rs"]
mod old_devices;
#[path = "full_client/reads.rs"]
mod reads;
#[path = "full_client/roles.rs"]
mod roles;
#[path = "full_client/routed_paths.rs"]
mod routed_paths;
#[path = "full_client/stop_waits.rs"]
mod stop_waits;

use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast};
use bacnet_encoding::apdu::{Apdu, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_endpoint::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::file::FileObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

use eventually::eventually;
use wire::Wire;

/// The node under test, its `bacnet-server` peer, and a third node.
const NODE: u8 = 1;
const PEER: u8 = 2;
const OTHER: u8 = 3;

const NODE_DEVICE: u32 = 1001;
const PEER_DEVICE: u32 = 2002;

const WAIT: Duration = Duration::from_secs(5);

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

/// The peer's file: 3,000 octets, each its position modulo 251.
fn file_octets() -> Vec<u8> {
    (0..3_000u32)
        .map(|position| (position % 251) as u8)
        .collect()
}

/// The node under test and its peers.
struct Lab {
    wire: Wire,
    session: EndpointSession<TestTransport>,
    /// What the node put on the link.
    node_sent: SendLog,
    node_link: TestTransportHandle,
    peer: BACnetServer<TestTransport>,
    /// What the peer put on the link.
    peer_sent: SendLog,
}

/// A started session on node [`NODE`], with device [`NODE_DEVICE`] and
/// AI-1 at 20.0, its read-only server when it has a server role, and the
/// peer server.
async fn lab(role: SessionRole) -> Lab {
    lab_with(role, false).await
}

/// [`lab`], on a link that takes the local Number controls when `numbers`.
async fn lab_with(role: SessionRole, numbers: bool) -> Lab {
    let wire = Wire::new();
    let peer_port = wire.port(PEER);
    let peer_sent = peer_port.sent();
    let peer = peer_server(peer_port.build()).await;

    let mut port = wire.port(NODE);
    if numbers {
        port = port.number_controls();
    }
    let (node_sent, node_link) = (port.sent(), port.handle());
    let identity = DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(20.0);
    let database = build_database_with_extra(&identity, vec![Box::new(input)]).unwrap();
    let mut session = EndpointSession::new(port.build(), role, SessionConfig::default())
        .unwrap()
        .device(identity)
        .with_database(database);
    session.start().await.unwrap();
    Lab {
        wire,
        session,
        node_sent,
        node_link,
        peer,
        peer_sent,
    }
}

/// Device [`PEER_DEVICE`]: AI-1 at 21.5, File-1 holding [`file_octets`],
/// and 400 analog values, so its Object_List is longer than one APDU.
async fn peer_server(transport: TestTransport) -> BACnetServer<TestTransport> {
    let identity = DeviceIdentity::new(PEER_DEVICE, 42)
        .unwrap()
        .with_segmentation(Segmentation::SEGMENTED_BOTH);
    let mut input =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(21.5);
    let mut file = FileObject::new(1, "Log", "raw").unwrap();
    file.set_data(file_octets());
    let mut objects: Vec<Box<dyn BACnetObject>> = vec![Box::new(input), Box::new(file)];
    for instance in 1..=400 {
        let value = AnalogValueObject::new(
            instance,
            format!("AV-{instance}"),
            EngineeringUnits::NO_UNITS,
        )
        .unwrap();
        objects.push(Box::new(value));
    }
    let database = build_database_with_extra(&identity, objects).unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(transport)
        .build()
        .await
        .unwrap()
}

/// A standalone client on node [`OTHER`], with a 1000 ms APDU timeout and
/// no retries, so a request nobody answers fails at once.
async fn other_client(wire: &Wire) -> BACnetClient<TestTransport> {
    BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap()
}

/// `apdu` in an NPDU with no addresses.
fn npdu_of(apdu: &Apdu, expecting_reply: bool) -> Bytes {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply,
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

/// An unsegmented Confirmed-Request for `service` under `invoke_id`.
fn confirmed(
    invoke_id: u8,
    service: bacnet_types::enums::ConfirmedServiceChoice,
    parameters: &[u8],
) -> Apdu {
    Apdu::ConfirmedRequest(bacnet_encoding::apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: Bytes::copy_from_slice(parameters),
    })
}

/// ReadProperty's parameters for `object`'s Object_Name.
fn read_name(object: ObjectIdentifier) -> Vec<u8> {
    let mut parameters = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: object,
        property_identifier: bacnet_types::enums::PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut parameters);
    parameters.to_vec()
}

/// The APDUs in `log`, each with the MAC it went to (empty for a
/// broadcast), leaving out network messages.
fn apdus(log: &SendLog) -> Vec<(MacAddr, Apdu)> {
    log.frames()
        .into_iter()
        .filter_map(|frame| {
            let npdu = decode_npdu(frame.npdu.clone()).ok()?;
            if npdu.message_type.is_some() {
                return None;
            }
            Some((frame.mac, decode_apdu(npdu.payload).ok()?))
        })
        .collect()
}

/// The network message types in `log`, in order.
fn network_messages(log: &SendLog) -> Vec<bacnet_npdu::NetworkMessageType> {
    log.frames()
        .into_iter()
        .filter_map(|frame| decode_npdu(frame.npdu).ok()?.message_type)
        .collect()
}

/// Stop the lab once its full client is dropped, and check the node's
/// transport stopped exactly once.
async fn stop(mut lab: Lab) {
    lab.session.stop().await.unwrap();
    assert_eq!(lab.node_link.stops(), 1);
    lab.peer.stop().await.unwrap();
}
