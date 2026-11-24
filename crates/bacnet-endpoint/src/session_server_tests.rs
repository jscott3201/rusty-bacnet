//! The full server a session builds (#2617), over an in-memory link with
//! broadcasts on paused time: the read-only server a session builds when it
//! is given no builder of its own, requests that are on the link before the
//! session starts, and a session dropped with its server where no runtime
//! runs.
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

/// The in-memory link with broadcasts, for tests of several nodes (#2617);
/// the client seat's tests take it from here too.
#[allow(dead_code)]
#[path = "../tests/support/wire.rs"]
pub(in crate::session) mod wire;
use wire::Wire;

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

fn ch5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, 5).unwrap()
}

/// The node's database: its Device, AV-7, AI-1, File-3 and Channel-5
/// (number 11, in control group 27).
fn database() -> ObjectDatabase {
    let identity = crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let units = EngineeringUnits::DEGREES_CELSIUS;
    let value = AnalogValueObject::new(7, "AV-7", units).unwrap();
    let input = AnalogInputObject::new(1, "AI-1", units).unwrap();
    let file = bacnet_objects::file::FileObject::new(3, "File-3", "text/plain").unwrap();
    let mut channel = bacnet_objects::channel::ChannelObject::new(5, "CH-5", 11).unwrap();
    channel.set_control_groups(vec![27]).unwrap();
    let extra: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(value),
        Box::new(input),
        Box::new(file),
        Box::new(channel),
    ];
    crate::build_database_with_extra(&identity, extra).unwrap()
}

/// A session of both roles on node [`NODE`] of `wire`, with Device
/// [`NODE_DEVICE`], the [`database`] and a full server from `builder`, not
/// started, and its send log.
fn session(
    wire: &Wire,
    builder: ServerBuilder<TestTransport>,
) -> (EndpointSession<TestTransport>, SendLog) {
    let (session, sent) = default_session(wire);
    (session.with_server(builder), sent)
}

/// [`session`] given no builder: it runs the read-only server.
fn default_session(wire: &Wire) -> (EndpointSession<TestTransport>, SendLog) {
    default_session_as(wire, SessionRole::Both)
}

/// [`default_session`] with `role`.
fn default_session_as(wire: &Wire, role: SessionRole) -> (EndpointSession<TestTransport>, SendLog) {
    let port = wire.port(NODE);
    let sent = port.sent();
    let session = EndpointSession::new(port.build(), role, SessionConfig::default())
        .unwrap()
        .device(crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
        .with_database(database());
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

/// The read-only server a session given no builder runs: a peer reads,
/// discovers the node and its objects and subscribes to COV, which the
/// server reports; no peer writes, creates or deletes, each refused
/// SERVICES / SERVICE_REQUEST_DENIED with nothing changed.
#[tokio::test(start_paused = true)]
async fn the_read_only_server_answers_reads_discovery_and_subscriptions_only() {
    let wire = Wire::new();
    let (mut session, _) = default_session(&wire);
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
    let changed = changed
        .into_object()
        .expect("a COV notification of one object");
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

/// A server-only session given no builder runs the read-only server too:
/// a peer reads and discovers the node, and its write is denied with
/// nothing changed.
#[tokio::test(start_paused = true)]
async fn a_server_only_session_runs_the_read_only_server() {
    let wire = Wire::new();
    let (mut session, _) = default_session_as(&wire, SessionRole::ServerOnly);
    session.start().await.unwrap();
    let peer = peer(&wire).await;
    let node = Wire::mac(NODE);
    let before = present_value(&peer).await;
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let written = peer.write_property(&node, av7(), pv, None, real(5.0), Some(8));
    assert!(denied(written.await));
    assert_eq!(present_value(&peer).await, before, "nothing changed");
    peer.who_is(Broadcast::Global, None).await.unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    eventually(WAIT, "the node's I-Am", async || {
        peer.discovered_devices()
            .await
            .iter()
            .any(|found| found.object_identifier == device)
    })
    .await;
    let mut peer = peer;
    peer.stop().await.unwrap();
    session.stop().await.unwrap();
}

/// A start cancelled after the session runs but before it builds its server
/// leaves a running session with no server and its database as it was,
/// and a stop then completes.
#[tokio::test(start_paused = true)]
async fn a_start_cancelled_before_its_server_is_built_leaves_none() {
    let wire = Wire::new();
    let (mut session, _) = default_session(&wire);
    let pauses = super::StartPauses {
        after_link: Duration::from_secs(5),
        ..Default::default()
    };
    let start = Box::pin(super::START_PAUSES.scope(pauses, session.start()));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), start)
            .await
            .is_err()
    );
    assert!(session.is_running(), "the cancelled start left it running");
    assert!(session.server().is_none(), "no server was built");
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let database = session.database.as_ref().unwrap().read().await;
    let value = database
        .get(&ai1())
        .unwrap()
        .read_property(pv, None)
        .unwrap();
    assert_eq!(value, PropertyValue::Real(0.0), "nothing changed");
    drop(database);
    let stopped = tokio::time::timeout(WAIT, session.stop()).await;
    assert!(stopped.is_ok(), "the stop completes");
    assert!(!session.is_running());
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
    assert_eq!(session.policy_counters().no_server_role, 0);
    assert_eq!(session.server_seat_counters().apdus, 1);
    session.stop().await.unwrap();
}

/// An application-tagged REAL.
fn real(value: f32) -> Vec<u8> {
    let mut bytes = vec![0x44];
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

/// What `server` serves for `object`'s `property`.
async fn served(
    server: &BACnetServer<TestTransport>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    server.read_local(&object, property, None).await.unwrap()
}

/// The rest of what the read-only server a session given no builder runs
/// refuses: WritePropertyMultiple, AddListElement, RemoveListElement,
/// AtomicWriteFile and ReinitializeDevice are answered SERVICES /
/// SERVICE_REQUEST_DENIED, and DeviceCommunicationControl with an Error
/// too, the node initiating as before; an inbound WriteGroup, which has no
/// answer, changes nothing. A TimeSynchronization or
/// UTCTimeSynchronization, unconfirmed too, leaves the Device's clock
/// alone.
#[tokio::test(start_paused = true)]
async fn the_read_only_server_refuses_the_other_mutations_and_time_sync() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::file::FileWriteAccessMethod;
    use bacnet_services::wpm::WriteAccessSpecification;
    use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
    use bacnet_types::enums::{EnableDisable, ReinitializedState};
    use bacnet_types::primitives::{Date, Time};
    let wire = Wire::new();
    let (mut session, _) = default_session(&wire);
    Box::pin(session.start()).await.unwrap();
    let peer = Box::pin(peer(&wire)).await;
    let node = Wire::mac(NODE);
    let server = session.server().unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let before = (
        served(server, av7(), pv).await,
        served(server, ch5(), pv).await,
    );
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let date = served(server, device, PropertyIdentifier::LOCAL_DATE).await;

    let specs = vec![WriteAccessSpecification {
        object_identifier: av7(),
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: pv,
            property_array_index: None,
            value: real(5.0),
            priority: Some(8),
        }],
    }];
    assert!(denied(
        Box::pin(peer.write_property_multiple(&node, specs)).await
    ));
    // An application-tagged Unsigned, 1: what the list holds doesn't matter.
    let list = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let added = peer.add_list_element(&node, ch5(), list, None, vec![0x21, 1]);
    assert!(denied(Box::pin(added).await));
    let removed = peer.remove_list_element(&node, ch5(), list, None, vec![0x21, 1]);
    assert!(denied(Box::pin(removed).await));
    let access = FileWriteAccessMethod::Stream {
        file_start_position: 0,
        file_data: b"written".to_vec(),
    };
    let file = ObjectIdentifier::new(ObjectType::FILE, 3).unwrap();
    assert!(denied(
        Box::pin(peer.atomic_write_file(&node, file, access)).await
    ));
    let group = WriteGroupRequest {
        group_number: std::num::NonZeroU32::new(27).unwrap(),
        write_priority: 8,
        change_list: vec![GroupChannelValue {
            channel: 11,
            override_priority: None,
            value: real(5.0),
        }],
        inhibit_delay: None,
    };
    peer.write_group_directed(&node, &group).await.unwrap();
    let reset = peer.reinitialize_device(&node, ReinitializedState::WARMSTART, None);
    assert!(denied(Box::pin(reset).await));
    let control =
        peer.device_communication_control(&node, EnableDisable::DISABLE_INITIATION, Some(5), None);
    assert!(denied(Box::pin(control).await));
    let new_year = Date {
        year: 101,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    let noon = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    peer.time_synchronization(&node, new_year, noon)
        .await
        .unwrap();
    peer.utc_time_synchronization(&node, new_year, noon)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = (
        served(server, av7(), pv).await,
        served(server, ch5(), pv).await,
    );
    assert_eq!(after, before, "nothing changed");
    let local = PropertyIdentifier::LOCAL_DATE;
    assert_eq!(served(server, device, local).await, date, "the clock moved");
    // The node still initiates: no DeviceCommunicationControl took hold.
    session.broadcast_i_am().await.unwrap();

    let mut peer = peer;
    peer.stop().await.unwrap();
    session.stop().await.unwrap();
}

/// Requests a peer sent before the session started are the server's
/// however late the session gets to them: the seat is attached inside the
/// link's start, before its first request is classified. A pause right
/// after the link starts gives the classifier its turn first; with the
/// seat taken after that, they would reach the session's own queue and
/// find no server.
#[tokio::test(start_paused = true)]
async fn requests_classified_as_the_link_starts_are_the_servers() {
    let wire = Wire::new();
    let (mut session, sent) = session(&wire, BACnetServer::generic_builder());
    for invoke_id in 1..=8 {
        wire.deliver(&Wire::mac(NODE), read_request(invoke_id));
    }
    let pauses = super::StartPauses {
        after_link: Duration::from_millis(20),
        ..Default::default()
    };
    Box::pin(super::START_PAUSES.scope(pauses, session.start()))
        .await
        .unwrap();
    eventually(WAIT, "eight answers", async || answers(&sent).len() == 8).await;
    assert!(
        answers(&sent)
            .iter()
            .all(|answer| matches!(answer, Apdu::ComplexAck(_))),
        "{:?}",
        answers(&sent)
    );
    assert_eq!(session.policy_counters().no_server_role, 0);
    assert_eq!(session.server_seat_counters().apdus, 8);
    session.stop().await.unwrap();
}

/// The Device's Protocol_Services_Supported as `peer` reads it from the node,
/// with the two time-synchronization bits (Clauses 16.7 and 16.8) apart from
/// the rest.
async fn services_by_peer(peer: &BACnetClient<TestTransport>) -> ([bool; 2], Vec<u8>) {
    use bacnet_types::enums::ServiceSupported;
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let ack = peer
        .read_property(
            &Wire::mac(NODE),
            device,
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap();
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    let bacnet_types::primitives::PropertyValue::BitString { mut data, .. } = value else {
        panic!("{value:?}")
    };
    let bits = [
        ServiceSupported::TIME_SYNCHRONIZATION,
        ServiceSupported::UTC_TIME_SYNCHRONIZATION,
    ]
    .map(|service| {
        let bit = usize::from(service.to_raw());
        let listed = data[bit / 8] & (0x80 >> (bit % 8)) != 0;
        data[bit / 8] &= !(0x80 >> (bit % 8));
        listed
    });
    (bits, data)
}

/// The read-only server's time synchronization is off, so its Device lists
/// neither time-synchronization service, and nothing else differs from a
/// server with the default policy, which lists both. A read-only server
/// given a policy that is on lists both again (#2617).
#[tokio::test(start_paused = true)]
async fn the_read_only_server_lists_no_time_synchronization() {
    let mut others = Vec::new();
    for (name, builder, expected) in [
        ("read-only", read_only_server(), [false; 2]),
        (
            "read-only, policy on",
            read_only_server().time_sync_policy(Default::default()),
            [true; 2],
        ),
        ("default", BACnetServer::generic_builder(), [true; 2]),
    ] {
        let wire = Wire::new();
        let (mut session, _) = session(&wire, builder);
        session.start().await.unwrap();
        let peer = peer(&wire).await;
        let (bits, rest) = services_by_peer(&peer).await;
        assert_eq!(bits, expected, "{name}");
        others.push(rest);
        session.stop().await.unwrap();
    }
    assert_eq!(others[0], others[2], "no other bit moves");
    assert_eq!(others[1], others[2]);
}

/// A session dropped with its server, unstopped, on a thread with no
/// runtime, whether the runtime that ran it is still there or gone, drops
/// cleanly: nothing in the drop needs a runtime.
#[test]
fn a_session_with_its_server_drops_where_no_runtime_runs() {
    for runtime_gone in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let session = runtime.block_on(async {
            let wire = Wire::new();
            let (mut session, _) = session(&wire, BACnetServer::generic_builder());
            session.start().await.unwrap();
            session
        });
        if runtime_gone {
            drop(runtime);
            std::thread::spawn(move || drop(session)).join().unwrap();
        } else {
            std::thread::spawn(move || drop(session)).join().unwrap();
            drop(runtime);
        }
    }
}

/// Refused builds and starts hand the database back (#2766).
#[path = "session_hand_back_tests.rs"]
mod hand_back;

/// Binary Input `instance`.
fn bi(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance).unwrap()
}

/// The encoded Present_Value that the next COV notification `heard`
/// brings about BI-2 carries.
async fn notified_present_value(
    heard: &mut tokio::sync::broadcast::Receiver<bacnet_client::client::COVEvent>,
) -> Vec<u8> {
    let event = tokio::time::timeout(WAIT, heard.recv())
        .await
        .expect("a COV notification")
        .unwrap();
    let received = event
        .into_object()
        .expect("a COV notification of one object");
    assert_eq!(received.notification.monitored_object_identifier, bi(2));
    received
        .notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .expect("Present_Value reported")
        .value
        .clone()
}

/// A Binary Input's physical side and its Polarity (#2594) are set through
/// the session's server, as its other local inputs are, and a peer's COV
/// subscription hears each change of Present_Value: an OFF input under
/// REVERSE is ACTIVE, and a Polarity change in service derives
/// Present_Value from it again. Out of service Present_Value is the
/// simulated one whatever the input or Polarity does, and the return to
/// service derives it from both (Clauses 12.6.4 and 12.6.10). Another
/// object, or an instance the node doesn't have, is refused.
#[tokio::test(start_paused = true)]
async fn a_binary_inputs_physical_side_and_polarity_reach_subscribers() {
    use bacnet_device_core::write::local::LocalCommandSource;
    use bacnet_objects::binary::BinaryInputObject;
    use bacnet_types::enums::Polarity;
    let (inactive, active) = (vec![0x91, 0], vec![0x91, 1]);
    let wire = Wire::new();
    let identity = crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut contact = BinaryInputObject::new(2, "BI-2").unwrap();
    contact.set_polarity(Polarity::REVERSE).unwrap();
    let units = EngineeringUnits::DEGREES_CELSIUS;
    let extra: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(contact),
        Box::new(AnalogInputObject::new(1, "AI-1", units).unwrap()),
    ];
    let database = crate::build_database_with_extra(&identity, extra).unwrap();
    let port = wire.port(NODE).build();
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(identity)
        .with_database(database);
    session.start().await.unwrap();
    let peer = peer(&wire).await;
    let mut heard = peer.cov_notifications();
    peer.subscribe_cov(&Wire::mac(NODE), 3, bi(2), false, Some(600))
        .await
        .unwrap();
    assert_eq!(notified_present_value(&mut heard).await, inactive);
    let server = session.server().unwrap();

    server
        .set_physical_input_local(&bi(2), false)
        .await
        .unwrap();
    assert_eq!(notified_present_value(&mut heard).await, active);
    let normal = Polarity::NORMAL;
    server.set_polarity_local(&bi(2), normal).await.unwrap();
    assert_eq!(notified_present_value(&mut heard).await, inactive);

    let write = async |property, value| {
        let source = LocalCommandSource::ServerDevice;
        server
            .write_local(&bi(2), property, None, value, None, source)
            .await
            .unwrap();
    };
    let present_value = async || {
        server
            .read_local(&bi(2), PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap()
    };
    let out_of_service = PropertyIdentifier::OUT_OF_SERVICE;
    write(out_of_service, PropertyValue::Boolean(true)).await;
    assert_eq!(notified_present_value(&mut heard).await, inactive);
    let simulated = PropertyValue::Enumerated(1);
    write(PropertyIdentifier::PRESENT_VALUE, simulated.clone()).await;
    assert_eq!(notified_present_value(&mut heard).await, active);
    server.set_physical_input_local(&bi(2), true).await.unwrap();
    let reverse = Polarity::REVERSE;
    server.set_polarity_local(&bi(2), reverse).await.unwrap();
    assert_eq!(present_value().await, simulated);
    // Back in service: ON under REVERSE.
    write(out_of_service, PropertyValue::Boolean(false)).await;
    assert_eq!(notified_present_value(&mut heard).await, inactive);

    let refused = |result: Result<(), Error>| match result {
        Err(error) => error.remote(),
        Ok(()) => panic!("taken"),
    };
    let object_error = |code| {
        Some(RemoteError::Error {
            class: ErrorClass::OBJECT,
            code,
        })
    };
    let unsupported = object_error(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED);
    let unknown = object_error(ErrorCode::UNKNOWN_OBJECT);
    for (oid, expected) in [(ai1(), unsupported), (bi(9), unknown)] {
        let input = server.set_physical_input_local(&oid, true).await;
        assert_eq!(refused(input), expected, "{oid}");
        let polarity = server.set_polarity_local(&oid, normal).await;
        assert_eq!(refused(polarity), expected, "{oid}");
    }
    assert_eq!(present_value().await, PropertyValue::Enumerated(0));

    let mut peer = peer;
    peer.stop().await.unwrap();
    session.stop().await.unwrap();
}
