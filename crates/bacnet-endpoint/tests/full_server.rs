//! A full `BACnetServer` and a full `BACnetClient` on one endpoint
//! session's link (#2617), on paused time over an in-memory link with
//! broadcasts.
//!
//! The node under test is an endpoint session with both roles, which builds
//! its full server at start ([`EndpointSession::with_server`]) on the
//! session's database (AI-1 at 42.0),
//! beside a full client. Its peers are a `bacnet-server` device and a
//! standalone client.
#![allow(dead_code)]

#[path = "support/eventually.rs"]
mod eventually;
#[path = "support/wire.rs"]
mod wire;

/// The node is one device: its DeviceCommunicationControl and its Device
/// object (#2617, PR 2).
#[path = "full_server/node.rs"]
mod node;

/// What a session refuses before its link starts when it would build a
/// full server (#2617).
#[path = "full_server/refusals.rs"]
mod refusals;

/// The node's Device lists what its database holds (#2669).
#[path = "full_server/object_list.rs"]
mod object_list;

use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast, COVNotificationDelivery};
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::constructed::encode_event_notification;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_endpoint::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_transport::test_transport::{SendLog, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, EventState, EventType, NotifyType, ObjectType,
    PropertyIdentifier,
};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};

use eventually::eventually;
use wire::Wire;

const NODE: u8 = 1;
const PEER: u8 = 2;
const OTHER: u8 = 3;

const NODE_DEVICE: u32 = 1001;
const PEER_DEVICE: u32 = 2002;

const WAIT: Duration = Duration::from_secs(5);

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// A database for `instance` with AI-1 at `value`.
fn database(instance: u32, value: f32) -> bacnet_objects::database::ObjectDatabase {
    let identity = DeviceIdentity::new(instance, 42).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(value);
    build_database_with_extra(&identity, vec![Box::new(input)]).unwrap()
}

/// A node with both roles, a full client and a full server on its link,
/// and two peers.
struct Lab {
    wire: Wire,
    session: EndpointSession<TestTransport>,
    node_sent: SendLog,
    node_link: TestTransportHandle,
    client: BACnetClient<TestTransport>,
    /// A device the node's client reads, with AI-1 at 21.5.
    peer: BACnetServer<TestTransport>,
    /// A standalone client that reads the node.
    other: BACnetClient<TestTransport>,
}

impl Lab {
    /// The node's full server, which the session owns.
    fn server(&self) -> &BACnetServer<TestTransport> {
        self.session.server().expect("the node's full server")
    }
}

/// A session on node [`NODE`] with `role`, Device [`NODE_DEVICE`], AI-1 at
/// 42.0 in its database and a full server built from `builder`, started.
async fn session(
    wire: &Wire,
    role: SessionRole,
    builder: bacnet_server::server::ServerBuilder<TestTransport>,
) -> (EndpointSession<TestTransport>, SendLog, TestTransportHandle) {
    let port = wire.port(NODE);
    let (sent, link) = (port.sent(), port.handle());
    let mut session = EndpointSession::new(port.build(), role, SessionConfig::default())
        .unwrap()
        .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
        .with_database(database(NODE_DEVICE, 42.0))
        .with_server(builder);
    session.start().await.unwrap();
    (session, sent, link)
}

async fn lab() -> Lab {
    let wire = Wire::new();
    let peer = BACnetServer::generic_builder()
        .device(DeviceIdentity::new(PEER_DEVICE, 42).unwrap())
        .database(database(PEER_DEVICE, 21.5))
        .transport(wire.port(PEER).build())
        .build()
        .await
        .unwrap();
    let other = BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let (session, node_sent, node_link) =
        session(&wire, SessionRole::Both, BACnetServer::generic_builder()).await;
    let client = session.full_client().unwrap();
    Lab {
        wire,
        session,
        node_sent,
        node_link,
        client,
        peer,
        other,
    }
}

/// The session's stop stops its server, then the link, once.
async fn stop(lab: Lab) {
    let Lab {
        mut session,
        node_link,
        client,
        mut peer,
        mut other,
        ..
    } = lab;
    drop(client);
    session.stop().await.unwrap();
    assert_eq!(node_link.stops(), 1);
    assert!(session.server().is_none(), "gone with the stop");
    other.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// The APDUs the node sent to `to`, leaving out network messages.
fn sent_to(log: &SendLog, to: &MacAddr) -> Vec<Apdu> {
    log.frames()
        .into_iter()
        .filter(|frame| frame.mac == *to)
        .filter_map(|frame| {
            let npdu = decode_npdu(frame.npdu).ok()?;
            if npdu.message_type.is_some() {
                return None;
            }
            decode_apdu(npdu.payload).ok()
        })
        .collect()
}

/// The invoke IDs of `log`'s Confirmed-Requests for `service`.
fn requests(log: &SendLog, service: ConfirmedServiceChoice) -> Vec<u8> {
    log.frames()
        .into_iter()
        .filter_map(|frame| decode_apdu(decode_npdu(frame.npdu).ok()?.payload).ok())
        .filter_map(|apdu| match apdu {
            Apdu::ConfirmedRequest(request) if request.service_choice == service => {
                Some(request.invoke_id)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_full_server_answers_for_the_node_beside_its_full_client() {
    let lab = lab().await;
    let node = Wire::mac(NODE);

    // The full server answers a Who-Is for the node's Device.
    lab.other.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the node's I-Am", async || {
        lab.other.discovered_devices().await.iter().any(|device| {
            device.object_identifier.instance_number() == NODE_DEVICE && device.mac_address == node
        })
    })
    .await;
    // It reads from the session's database, and it runs the services the
    // 0.15.0 endpoint's narrow server role didn't, ReadPropertyMultiple among them.
    let value = lab
        .other
        .read_property(&node, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    // An application-tagged REAL: tag 4, length 4.
    let mut real = vec![0x44];
    real.extend_from_slice(&42.0f32.to_be_bytes());
    assert_eq!(value.property_value, real);
    let many = lab
        .other
        .read_property_multiple(
            &node,
            vec![ReadAccessSpecification {
                object_identifier: ai1(),
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                ],
            }],
        )
        .await
        .unwrap();
    assert_eq!(many.list_of_read_access_results[0].list_of_results.len(), 2);

    // The node's client reads the peer meanwhile.
    let peer_value = lab
        .client
        .read_property(
            &Wire::mac(PEER),
            ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();
    assert_eq!(peer_value.object_identifier, ai1());

    // No request found the node without a server.
    let counters = lab.session.policy_counters().await;
    assert_eq!(counters.no_server_role, 0);
    assert_eq!(counters.confirmed_full_drops, 0);
    let seat = lab.session.server_seat_counters();
    assert!(seat.apdus >= 3, "{seat:?}");
    assert_eq!(seat.full_drops, 0);

    stop(lab).await;
}

/// A ConfirmedEventNotification's parameters.
fn event_notification() -> Vec<u8> {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 3003).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("too hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    };
    let mut parameters = BytesMut::new();
    encode_event_notification(&request, &mut parameters).unwrap();
    parameters.to_vec()
}

/// `service` with `parameters` under `invoke_id`, in an NPDU expecting a
/// reply.
fn confirmed(invoke_id: u8, service: ConfirmedServiceChoice, parameters: &[u8]) -> Bytes {
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
            service_choice: service,
            service_request: Bytes::copy_from_slice(parameters),
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
    npdu.freeze()
}

#[tokio::test(start_paused = true)]
async fn the_server_acknowledges_an_event_notification_the_client_hears() {
    let lab = lab().await;
    let (node, other) = (Wire::mac(NODE), Wire::mac(OTHER));
    let mut events = lab.client.event_notifications();
    // The narrow server role rejected both of these as services it doesn't
    // know (`full_client/roles.rs` pins that); the full server runs them.
    let requests = [
        // File 9, which the node hasn't: stream access from 0, 16 octets.
        confirmed(
            1,
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            &[
                0xc4, 0x02, 0x80, 0x00, 0x09, 0x0e, 0x31, 0x00, 0x21, 0x10, 0x0f,
            ],
        ),
        confirmed(
            2,
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            &event_notification(),
        ),
    ];
    for request in requests {
        lab.wire.deliver(&node, wire::frame(&other, request, false));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let answers = sent_to(&lab.node_sent, &other);
    let [Apdu::Error(file), Apdu::SimpleAck(event)] = &answers[..] else {
        panic!("the full server's answers: {answers:?}");
    };
    assert_eq!(file.invoke_id, 1);
    assert_eq!(
        (event.invoke_id, event.service_choice),
        (2, ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
    );
    // The client heard the event notification, and answered nothing.
    let heard = events.try_recv().expect("the event notification");
    assert_eq!(heard.source_mac, other);
    assert!(events.try_recv().is_err());

    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn each_answer_goes_to_the_role_that_asked() {
    let lab = lab().await;
    let (node, peer, other) = (Wire::mac(NODE), Wire::mac(PEER), Wire::mac(OTHER));

    // The other client subscribes to the node's AI-1 with confirmed
    // notifications, which the full server sends and that client
    // acknowledges; the node's client subscribes to the peer's AI-1 the
    // same way.
    let mut from_node = lab.other.cov_notifications();
    let mut from_peer = lab.client.cov_notifications();
    // Each notification arrives, confirmed, and is acknowledged before the
    // next change.
    let acknowledged = async |heard: &mut tokio::sync::broadcast::Receiver<_>, what: &str| {
        let notification: bacnet_client::client::COVEvent =
            tokio::time::timeout(WAIT, heard.recv())
                .await
                .unwrap_or_else(|_| panic!("no notification {what}"))
                .unwrap();
        assert_eq!(notification.delivery(), COVNotificationDelivery::Confirmed);
        eventually(WAIT, "the notification acknowledged", async || {
            lab.session.active_leases() == 0
        })
        .await;
    };
    lab.other
        .subscribe_cov(&node, 7, ai1(), true, Some(600))
        .await
        .unwrap();
    acknowledged(&mut from_node, "from the node").await;
    lab.client
        .subscribe_cov(&peer, 9, ai1(), true, Some(600))
        .await
        .unwrap();
    acknowledged(&mut from_peer, "from the peer").await;
    for round in 0..5u8 {
        lab.server()
            .set_present_value_local(&ai1(), PropertyValue::Real(50.0 + f32::from(round)))
            .await
            .unwrap();
        acknowledged(&mut from_node, "from the node").await;
        lab.peer
            .set_present_value_local(&ai1(), PropertyValue::Real(60.0 + f32::from(round)))
            .await
            .unwrap();
        acknowledged(&mut from_peer, "from the peer").await;
    }
    // Long enough for any unanswered notification to be sent again.
    tokio::time::sleep(Duration::from_secs(20)).await;
    let notifications = |log| requests(log, ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION);
    // Each acknowledgment reached the server that sent the notification,
    // so none went twice; and the node's client acknowledged each of the
    // peer's once, with nothing from the node's server.
    assert_eq!(notifications(&lab.node_sent).len(), 6);
    assert_eq!(
        notifications(&lab.peer.transport().unwrap().sent()).len(),
        6
    );
    let to_peer: Vec<Apdu> = sent_to(&lab.node_sent, &peer)
        .into_iter()
        .filter(|apdu| !matches!(apdu, Apdu::ConfirmedRequest(_)))
        .collect();
    assert_eq!(to_peer.len(), 6, "{to_peer:?}");
    assert!(to_peer.iter().all(|apdu| matches!(
        apdu,
        Apdu::SimpleAck(ack)
            if ack.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
    )));
    // The node's notifications and the client's requests drew on one pool,
    // so nothing was claimed by a role that didn't ask.
    assert_eq!(lab.session.policy_counters().await.unclaimed_terminal, 0);
    assert!(sent_to(&lab.node_sent, &other).len() >= 6);

    stop(lab).await;
}

/// `server()` is the session's while it runs: `None` before start, for a
/// session without a server role and after stop. A session given no
/// builder runs the read-only server.
#[tokio::test(start_paused = true)]
async fn the_server_is_borrowed_while_the_session_runs() {
    let wire = Wire::new();
    let port = wire.port(NODE);
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
            .with_server(BACnetServer::generic_builder());
    assert!(session.server().is_none(), "not before start");
    session.start().await.unwrap();
    assert!(session.server().is_some());
    session.stop().await.unwrap();
    assert!(session.server().is_none(), "not after stop");

    let port = wire.port(PEER);
    let mut default =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(DeviceIdentity::new(PEER_DEVICE, 42).unwrap());
    default.start().await.unwrap();
    assert!(default.server().is_some(), "the read-only server");
    default.stop().await.unwrap();

    let port = wire.port(OTHER);
    let mut client = EndpointSession::new(
        port.build(),
        SessionRole::ClientOnly,
        SessionConfig::default(),
    )
    .unwrap();
    client.start().await.unwrap();
    assert!(client.server().is_none(), "no server role");
    client.stop().await.unwrap();
}

/// The node's AI-1 as `reader` reads it.
async fn read_node(reader: &BACnetClient<TestTransport>) -> f32 {
    let value = tokio::time::timeout(
        WAIT,
        reader.read_property(
            &Wire::mac(NODE),
            ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("an answer from the node")
    .unwrap();
    f32::from_be_bytes(value.property_value[1..5].try_into().unwrap())
}
