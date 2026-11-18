//! A server on a node's seat serves the node's Device and holds the node's
//! DeviceCommunicationControl state (#2617, PR 2): the node is one device.
use super::tests::{Node, PEER, answered, answers, node};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_objects::DeviceIdentity;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{EnableDisable, ServiceSupported};

const DEVICE: u32 = 1234;

/// A node with a client role whose client takes APDUs of up to 480 octets,
/// and one APDU timeout and retry count for both roles.
fn settings(client: bool) -> NodeDevice {
    NodeDevice {
        client,
        client_max_apdu_length: 480,
        apdu_timeout_ms: 2_000,
        apdu_retries: 1,
    }
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(DEVICE, 42).unwrap()
}

fn device() -> ObjectIdentifier {
    identity().device_oid()
}

/// A seat that carries the node's settings, and its builder.
async fn seated_builder(client: bool) -> (Node, SeatedServerBuilder<TestTransport>) {
    let (node, mut seat) = node().await;
    seat.node = Some(settings(client));
    let builder = BACnetServer::generic_builder()
        .seat(seat)
        .node_identity(identity());
    (node, builder)
}

/// `request` as the link hands it up from [`PEER`], under `invoke_id`.
fn from_peer(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    request: &[u8],
) -> ReceivedNpdu {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: Bytes::copy_from_slice(request),
        }),
    )
    .unwrap();
    let mut npdu = vec![0x01, 0x04];
    npdu.extend_from_slice(&apdu);
    ReceivedNpdu {
        direct_response: None,
        npdu: Bytes::from(npdu),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

async fn unsigned(server: &BACnetServer<TestTransport>, property: PropertyIdentifier) -> u64 {
    match value(server, property).await {
        PropertyValue::Unsigned(value) => value,
        other => panic!("{property:?}: {other:?}"),
    }
}

async fn value(
    server: &BACnetServer<TestTransport>,
    property: PropertyIdentifier,
) -> PropertyValue {
    server.read_local(&device(), property, None).await.unwrap()
}

/// The Protocol_Services_Supported bits the server serves.
async fn services(server: &BACnetServer<TestTransport>) -> Vec<u8> {
    match value(server, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED).await {
        PropertyValue::BitString { data, .. } => data,
        other => panic!("services: {other:?}"),
    }
}

/// Whether the Protocol_Services_Supported bits in `data` hold `service`.
fn lists(data: &[u8], service: ServiceSupported) -> bool {
    let bit = usize::from(service.to_raw());
    data.get(bit / 8)
        .is_some_and(|octet| octet & (0x80 >> (bit % 8)) != 0)
}

/// The Device of a node with a client role states the node: the client's
/// executed services beside the server's, the smaller largest APDU, and the
/// one timing both roles use, which the server's notifications use too.
#[tokio::test(start_paused = true)]
async fn the_node_device_states_what_the_whole_node_does() {
    let (node, builder) = seated_builder(true).await;
    let server = builder.build().await.unwrap();

    let data = services(&server).await;
    for service in [
        ServiceSupported::I_AM,
        ServiceSupported::I_HAVE,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::READ_PROPERTY,
        ServiceSupported::CONFIRMED_EVENT_NOTIFICATION,
    ] {
        assert!(lists(&data, service), "{service:?}");
    }
    assert_eq!(
        unsigned(&server, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED).await,
        480
    );
    assert_eq!(
        unsigned(&server, PropertyIdentifier::APDU_TIMEOUT).await,
        2_000
    );
    assert_eq!(
        unsigned(&server, PropertyIdentifier::NUMBER_OF_APDU_RETRIES).await,
        1
    );
    assert_eq!(
        (
            server.config.cov_retry_timeout_ms,
            server.config.notification_retries,
            server.notification_transactions.retries()
        ),
        (2_000, 1, 1)
    );

    // A peer's ReadProperty is served from the same view.
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
        property_array_index: None,
    }
    .encode(&mut request);
    node.inbound
        .send(from_peer(
            9,
            ConfirmedServiceChoice::READ_PROPERTY,
            &request,
        ))
        .await
        .unwrap();
    let replies = answered(&node.sent, 1).await;
    let [Apdu::ComplexAck(ack)] = &replies[..] else {
        panic!("{replies:?}")
    };
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    let (served, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    let PropertyValue::BitString { data: served, .. } = served else {
        panic!("{served:?}")
    };
    assert_eq!(served, data);
}

/// A node with no client role lists the server's services alone.
#[tokio::test(start_paused = true)]
async fn a_server_only_node_lists_the_servers_services() {
    let (_node, builder) = seated_builder(false).await;
    let server = builder.build().await.unwrap();
    let data = services(&server).await;
    assert!(lists(&data, ServiceSupported::READ_PROPERTY));
    for service in [
        ServiceSupported::I_AM,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
    ] {
        assert!(!lists(&data, service), "{service:?}");
    }
    assert_eq!(
        unsigned(&server, PropertyIdentifier::APDU_TIMEOUT).await,
        2_000
    );
}

/// A seated build refuses what contradicts the node rather than picking
/// one: the node's identity, timing, and the Device a database declares.
#[tokio::test(start_paused = true)]
async fn a_seated_build_refuses_settings_that_contradict_the_node() {
    async fn refused(builder: SeatedServerBuilder<TestTransport>, what: &str) {
        let error = builder.build().await.err().expect(what);
        assert!(error.to_string().contains(what), "{what}: {error}");
    }

    let (_node, mut seat) = node().await;
    seat.node = Some(settings(true));
    refused(
        BACnetServer::generic_builder().seat(seat),
        "Device identity",
    )
    .await;

    let (_node, mut seat) = node().await;
    seat.node = Some(settings(true));
    let other = DeviceIdentity::new(DEVICE + 1, 42).unwrap();
    refused(
        BACnetServer::generic_builder()
            .device(other)
            .seat(seat)
            .node_identity(identity()),
        "Device identity",
    )
    .await;

    let (_node, mut builder) = seated_builder(true).await;
    builder.config.cov_retry_timeout_ms = 5_000;
    refused(builder, "APDU timeout").await;

    let (_node, mut builder) = seated_builder(true).await;
    builder.config.notification_retries = 7;
    refused(builder, "APDU retries").await;

    // A Device that declares a timing of its own.
    let (_node, mut builder) = seated_builder(true).await;
    let mut db = identity()
        .with_max_apdu(480)
        .unwrap()
        .build_database()
        .unwrap();
    db.get_mut(&device())
        .and_then(|object| object.device_authority_internal())
        .unwrap()
        .set_apdu_timing(5_000, 2);
    builder.db = Some(db);
    refused(builder, "APDU_Timeout").await;

    // One built with the object model's defaults (6000 ms, 3), which an
    // identity-built database can't change, is given the node's.
    let (_node, mut builder) = seated_builder(true).await;
    builder.db = Some(
        identity()
            .with_max_apdu(480)
            .unwrap()
            .build_database()
            .unwrap(),
    );
    let server = builder.build().await.expect("defaults are the node's");
    assert_eq!(
        unsigned(&server, PropertyIdentifier::APDU_TIMEOUT).await,
        2_000
    );

    // What agrees is taken: the node's own identity set on the builder, and
    // a Device that declares the node's largest APDU and timing.
    let mut db = identity()
        .with_max_apdu(480)
        .unwrap()
        .build_database()
        .unwrap();
    db.get_mut(&device())
        .and_then(|object| object.device_authority_internal())
        .unwrap()
        .set_apdu_timing(2_000, 1);
    let (_node, mut seat) = node().await;
    seat.node = Some(settings(true));
    BACnetServer::generic_builder()
        .device(identity())
        .database(db)
        .seat(seat)
        .node_identity(identity())
        .build()
        .await
        .expect("agreeing settings");
}

/// A DeviceCommunicationControl request with `state` for `minutes`.
fn dcc(invoke_id: u8, state: EnableDisable, minutes: Option<u16>) -> ReceivedNpdu {
    let mut request = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: minutes,
        enable_disable: state,
        password: None,
    }
    .encode(&mut request)
    .unwrap();
    from_peer(
        invoke_id,
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        &request,
    )
}

/// The seated server sets the node's state as it accepts a control, and
/// the node follows it back to ENABLE when the duration runs out and when
/// the server stops.
#[tokio::test(start_paused = true)]
async fn the_seated_server_holds_the_node_state() {
    let (node, seat) = node().await;
    let state = node.ingress.node_dcc().unwrap();
    let mut server = BACnetServer::generic_builder()
        .dcc_policy(DccPolicy::LegacyPermissive)
        .seat(seat)
        .build()
        .await
        .unwrap();

    node.inbound
        .send(dcc(1, EnableDisable::DISABLE_INITIATION, Some(1)))
        .await
        .unwrap();
    answered(&node.sent, 1).await;
    assert!(state.is_restricted());
    assert_eq!(server.comm_state(), DccState::DisableInitiation);
    // The duration runs out for the server and the node at once.
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(!state.is_restricted());
    assert_eq!(server.comm_state(), DccState::Enable);

    node.inbound
        .send(dcc(2, EnableDisable::DISABLE_INITIATION, None))
        .await
        .unwrap();
    answered(&node.sent, 2).await;
    assert!(state.is_restricted());
    // Gone with the server, as at a restart.
    server.stop().await.unwrap();
    assert!(!state.is_restricted());
    assert!(
        answers(&node.sent)
            .iter()
            .all(|answer| matches!(answer, Apdu::SimpleAck(_))),
        "{:?}",
        answers(&node.sent)
    );
}

/// Target Audit on a seat (#2617, PR 2): a write the server executes is
/// recorded and its notification goes out on the node's link; and when the
/// node's Number owner publishes the network's number, the recipient is
/// routed again. Here the Logger is bound through a router to network 5 at
/// the link's broadcast MAC, which names no single device once 5 is known to
/// be this network, so the Reporter then reports CONFIGURATION_ERROR.
#[tokio::test(start_paused = true)]
async fn a_seated_server_audits_and_follows_the_nodes_number() {
    use bacnet_objects::audit::AuditReporterObject;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_types::bitstring::AuditOperationFlags;
    use bacnet_types::enums::{AuditLevel, AuditOperation, Reliability};

    const ROUTER: [u8; 1] = [0x33];
    let (transport, inbound) = {
        let (tx, rx) = mpsc::channel(8);
        let transport = TestTransport::builder()
            .broadcast_mac(&[0xFF])
            .inbound(rx)
            .build();
        (transport, tx)
    };
    let sent = transport.sent();
    let mut ingress = bacnet_endpoint_core::endpoint_ingress::EndpointIngress::new(transport, 8);
    let receivers = ingress.start().await.unwrap();
    let seat = ingress
        .server_seat(Arc::new(
            bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator::new(),
        ))
        .unwrap();
    let network = Arc::clone(&seat.network);

    let target = DeviceIdentity::new(10, 0).unwrap();
    let logger = ObjectIdentifier::new(ObjectType::DEVICE, 20).unwrap();
    let reporter_oid = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();
    let value = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    let mut db = target.build_database().unwrap();
    db.get_mut(&target.device_oid())
        .and_then(|device| device.device_authority_internal())
        .unwrap()
        .provision_audit_recipient(bacnet_types::constructed::BACnetRecipient::Device(logger))
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations).unwrap();
    db.add(Box::new(reporter)).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
        .unwrap();

    let server = BACnetServer::generic_builder()
        .device(target)
        .database(db)
        .audit_reporters(AuditReportersConfig {
            reporters: vec![reporter_oid],
        })
        .device_binding(DeviceBinding::routed(logger, 5, [0xFF], ROUTER).unwrap())
        .unwrap()
        .seat(seat)
        .build()
        .await
        .expect("a seat takes Audit Reporters");

    let mut request = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: value,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: vec![0x91, 1],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    inbound
        .send(from_peer(
            3,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &request,
        ))
        .await
        .unwrap();
    let mut recorded = false;
    for _ in 0..5_000 {
        recorded = sent.frames().iter().any(|frame| {
            frame.mac.as_slice() == ROUTER
                && bacnet_encoding::npdu::decode_npdu(frame.npdu.clone())
                    .ok()
                    .and_then(|npdu| apdu::decode_apdu(npdu.payload).ok())
                    .is_some_and(|apdu| {
                        matches!(apdu, Apdu::UnconfirmedRequest(request)
                            if request.service_choice
                                == UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION)
                    })
        });
        if recorded {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(recorded, "no audit notification: {:?}", sent.frames());
    let reliability = || async {
        server
            .read_local(&reporter_oid, PropertyIdentifier::RELIABILITY, None)
            .await
            .unwrap()
    };
    assert_eq!(
        reliability().await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );

    // The node's Number owner publishes 5: the binding is local now, at a
    // broadcast MAC, so the recipient has no route.
    network
        .local_network_number()
        .publish(bacnet_types::network_number::NetworkNumber::configured(5).unwrap());
    receivers.seat_number.published();
    let mut faulted = false;
    for _ in 0..1_000 {
        faulted = reliability().await
            == PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw());
        if faulted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(faulted, "the Reporter did not follow the node's number");
}
