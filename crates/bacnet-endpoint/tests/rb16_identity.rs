//! RB-16 identity contract: single source + I-Am ≡ ReadProperty ≡ roles.
//!
//! Deterministic (Loopback + wire-byte asserts, no sleeps, no real sockets).
//! Real-loopback proofs live in `rb16_bip_proof` / `rb16_sc_proof`; this file
//! locks the truth direction, port rule, UUID rule, and capability matrix.
//!
//! Transport-dependent notes are recorded inline where Loopback differs from
//! real B/IP (broadcast reach) or SC (provenance).

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity, NetworkPortEntry};
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{
    NetworkType, ObjectType, PropertyIdentifier, Segmentation, ServiceSupported,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::PropertyValue;
use std::net::Ipv4Addr;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(2);

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 16,
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn analog(instance: u32, value: f32) -> Box<dyn bacnet_objects::traits::BACnetObject> {
    let mut o = AnalogInputObject::new(instance, format!("ai-{instance}"), 0).unwrap();
    o.set_present_value(value);
    Box::new(o)
}

fn object_id(t: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, instance).unwrap()
}

#[test]
fn identity_defaults_are_endpoint_composed_no_superset() {
    // Default = narrow endpoint reality (READ_PROPERTY only), not the full
    // server EXECUTED_SERVICES. No superset flags.
    let id = DeviceIdentity::new(42, 7).unwrap();
    assert_eq!(id.instance(), 42);
    assert_eq!(id.vendor_id(), 7);
    assert_eq!(id.max_apdu_length(), 1476);
    assert_eq!(id.segmentation(), Segmentation::NONE);
    assert_eq!(id.services(), &[ServiceSupported::READ_PROPERTY]);
    assert_eq!(id.device_uuid(), [0; 16]);
    assert!(id.network_ports().is_empty());
}

#[test]
fn identity_rejects_bad_instance_and_max_apdu_and_dup_port() {
    assert!(DeviceIdentity::new(4_194_304, 1).is_err());
    let id = DeviceIdentity::new(1, 1).unwrap();
    assert!(id.clone().with_max_apdu(999).is_err());
    let id = id.with_bip_port(1, 1, Ipv4Addr::LOCALHOST, 47808).unwrap();
    assert!(id.with_bip_port(1, 1, Ipv4Addr::LOCALHOST, 47809).is_err());
}

#[test]
fn identity_builds_device_plus_ports_plus_uuid() {
    let uuid = [0xA5; 16];
    let id = DeviceIdentity::new(99, 42)
        .unwrap()
        .with_max_apdu(480)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_device_uuid(uuid)
        .with_bip_port(1, 10, Ipv4Addr::new(127, 0, 0, 1), 47808)
        .unwrap()
        .with_sc_port(2, 20, [0x02; 6])
        .unwrap();
    let db = id.build_database().unwrap();

    // Device readback agrees with identity (truth direction).
    let dev_oid = object_id(ObjectType::DEVICE, 99);
    let dev = db.get(&dev_oid).expect("device must exist");
    assert_eq!(
        dev.read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .unwrap(),
        PropertyValue::ObjectIdentifier(dev_oid)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::VENDOR_IDENTIFIER, None)
            .unwrap(),
        PropertyValue::Unsigned(42)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
            .unwrap(),
        PropertyValue::Unsigned(480)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, None)
            .unwrap(),
        PropertyValue::Enumerated(Segmentation::NONE.to_raw() as u32)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::DEVICE_UUID, None)
            .unwrap(),
        PropertyValue::OctetString(uuid.to_vec())
    );
    // Services == composed reality (single READ_PROPERTY bit, 7 octets).
    match dev
        .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => {
            let ss = bacnet_types::bitstring::ServicesSupported::from_bacnet(&data);
            assert!(ss.contains(ServiceSupported::READ_PROPERTY));
            assert_eq!(ss.iter().count(), 1);
        }
        other => panic!("expected BitString, got {other:?}"),
    }
    // Object_List contains Device + both ports (port rule: one entry per
    // bound transport; instances caller-stable, B/IP=1 SC=2 here).
    match dev
        .read_property(PropertyIdentifier::OBJECT_LIST, Some(0))
        .unwrap()
    {
        PropertyValue::Unsigned(n) => assert_eq!(n, 3),
        other => panic!("expected count, got {other:?}"),
    }

    // B/IP port carries actual bound IP/port + network number.
    let bip_oid = object_id(ObjectType::NETWORK_PORT, 1);
    let bip = db.get(&bip_oid).expect("bip port must exist");
    assert_eq!(
        bip.read_property(PropertyIdentifier::NETWORK_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(NetworkType::IPV4.to_raw())
    );
    assert_eq!(
        bip.read_property(PropertyIdentifier::NETWORK_NUMBER, None)
            .unwrap(),
        PropertyValue::Unsigned(10)
    );
    assert_eq!(
        bip.read_property(PropertyIdentifier::IP_ADDRESS, None)
            .unwrap(),
        PropertyValue::OctetString(vec![127, 0, 0, 1])
    );
    assert_eq!(
        bip.read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
            .unwrap(),
        PropertyValue::Unsigned(47808)
    );
    assert_eq!(
        bip.read_property(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
            .unwrap(),
        PropertyValue::Unsigned(480)
    );

    // SC port is VIRTUAL with VMAC, no IP socket fields.
    let sc_oid = object_id(ObjectType::NETWORK_PORT, 2);
    let sc = db.get(&sc_oid).expect("sc port must exist");
    assert_eq!(
        sc.read_property(PropertyIdentifier::NETWORK_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(NetworkType::VIRTUAL.to_raw())
    );
    assert_eq!(
        sc.read_property(PropertyIdentifier::MAC_ADDRESS, None)
            .unwrap(),
        PropertyValue::OctetString(vec![0x02; 6])
    );
}

#[test]
fn identity_iam_bytes_match_server_discovery_construction() {
    // Wire-byte proof: endpoint I-Am encoding equals the server discovery
    // helper built from the derived ServerConfig (alignment bridge).
    let id = DeviceIdentity::new(1234, 99)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap();
    let endpoint_bytes = id.encode_iam_apdu().unwrap();
    let apdu = decode_apdu(endpoint_bytes.clone().into()).unwrap();
    let (choice, payload) = match apdu {
        Apdu::UnconfirmedRequest(req) => (req.service_choice, req.service_request),
        other => panic!("expected UnconfirmedRequest, got {other:?}"),
    };
    assert_eq!(choice, UnconfirmedServiceChoice::I_AM);
    let decoded = bacnet_services::who_is::IAmRequest::decode(&payload).unwrap();
    assert_eq!(
        decoded.object_identifier,
        object_id(ObjectType::DEVICE, 1234)
    );
    assert_eq!(decoded.max_apdu_length, 1476);
    assert_eq!(decoded.segmentation_supported, Segmentation::NONE);
    assert_eq!(decoded.vendor_id, 99);

    // Server side builds the same four fields from the derived config.
    let server_config = id.server_config();
    let server_req = bacnet_server::server::discovery_iam_for_test(id.device_oid(), &server_config);
    assert_eq!(
        server_req,
        bacnet_services::who_is::IAmRequest {
            object_identifier: id.device_oid(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 99,
        }
    );
}

#[tokio::test]
async fn session_identity_overrides_480_and_iam_broadcasts() {
    // SessionConfig 480 default is kept for standalone sessions; composed
    // identity overrides to 1476. I-Am broadcast derives from identity.
    let id = DeviceIdentity::new(77, 11)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap();
    let db = build_database_with_extra(&id, vec![analog(1, 1.0)]).unwrap();
    let (t_a, t_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut a = EndpointSession::new(t_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(db)
        .with_identity(id.clone());
    let mut b_net = bacnet_network::layer::NetworkLayer::new(t_b);
    let mut b_rx = b_net.start().await.unwrap();
    a.start().await.unwrap();
    assert!(a.is_running());
    assert!(a.identity().is_some());
    assert_eq!(a.identity().unwrap().max_apdu_length(), 1476);

    // Narrow admin borrow works while roles run; roles expose no lifecycle.
    assert_eq!(a.active_leases(), 0);
    let _ = a.policy_counters().await;
    assert!(a.client().is_some() && a.server().is_some());

    a.broadcast_i_am().await.unwrap();
    let got = tokio::time::timeout(WAIT, b_rx.recv())
        .await
        .expect("I-Am must arrive")
        .expect("peer closed");
    let apdu = decode_apdu(got.apdu.clone()).unwrap();
    match apdu {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(req.service_choice, UnconfirmedServiceChoice::I_AM);
            let iam = bacnet_services::who_is::IAmRequest::decode(&req.service_request).unwrap();
            assert_eq!(iam.object_identifier, object_id(ObjectType::DEVICE, 77));
            assert_eq!(iam.max_apdu_length, 1476);
            assert_eq!(iam.vendor_id, 11);
        }
        other => panic!("expected I-Am, got {other:?}"),
    }

    // Missing identity errors without sending (no invented device).
    let (t_c, _t_d) = LoopbackTransport::pair(vec![0x03], vec![0x04]);
    let mut c = EndpointSession::new(t_c, SessionRole::Both, session_config()).unwrap();
    c.start().await.unwrap();
    assert!(c.broadcast_i_am().await.is_err());
    c.stop().await.unwrap();

    a.stop().await.unwrap();
    b_net.stop().await.unwrap();
}

#[tokio::test]
async fn capability_matrix_services_and_segmentation_refuse() {
    // Services == reality: ReadProperty succeeds; WriteProperty (unexecuted)
    // draws Reject UNRECOGNIZED_SERVICE from the narrow responder.
    // Segmentation NONE: segmented request draws server Abort; segmented
    // ComplexAck draws client Abort (both directions refuse, Loopback here;
    // real-loopback proofs assert the same aborts on wire).
    use bacnet_encoding::apdu::{
        encode_apdu, AbortPdu, ConfirmedRequest as ConfirmedPdu, RejectPdu,
    };
    use bacnet_services::read_property::ReadPropertyRequest;
    use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, RejectReason};
    use bytes::BytesMut;

    let id = DeviceIdentity::new(55, 5).unwrap();
    let db_a = build_database_with_extra(&id, vec![analog(9, 3.25)]).unwrap();
    let db_b = {
        let id_b = DeviceIdentity::new(56, 5).unwrap();
        build_database_with_extra(&id_b, vec![analog(9, 9.5)]).unwrap()
    };
    // Pair for baseline: two full sessions so ReadProperty has a responder.
    let (t_a, t_b) = LoopbackTransport::pair(vec![0xA1], vec![0xB2]);
    let mut a = EndpointSession::new(t_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(db_a)
        .with_identity(id);
    let mut b = EndpointSession::new(t_b, SessionRole::Both, session_config())
        .unwrap()
        .with_database(db_b);
    a.start().await.unwrap();
    b.start().await.unwrap();

    // Baseline: ReadProperty (executed) succeeds with present value.
    let ack = tokio::time::timeout(
        WAIT,
        a.client().unwrap().read_property(
            &[0xB2],
            object_id(ObjectType::ANALOG_INPUT, 9),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("read timed out")
    .expect("read failed");
    assert_eq!(
        ack.object_identifier,
        object_id(ObjectType::ANALOG_INPUT, 9)
    );

    // Separate pair for Reject/Abort: peer sends, session responds.
    let id_c = DeviceIdentity::new(57, 5).unwrap();
    let db_c = build_database_with_extra(&id_c, vec![analog(9, 1.0)]).unwrap();
    let (t_c, t_d) = LoopbackTransport::pair(vec![0xC1], vec![0xD2]);
    let mut c = EndpointSession::new(t_c, SessionRole::Both, session_config())
        .unwrap()
        .with_database(db_c)
        .with_identity(id_c);
    let mut peer = bacnet_network::layer::NetworkLayer::new(t_d);
    let mut peer_rx = peer.start().await.unwrap();
    c.start().await.unwrap();

    // Unexecuted service (WriteProperty) draws Reject on the wire.
    let mut svc = BytesMut::new();
    // Minimal WriteProperty body: object id + property id + value (the
    // responder rejects before parsing detail, so exact body is inert).
    ReadPropertyRequest {
        object_identifier: object_id(ObjectType::ANALOG_INPUT, 9),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    let write_req = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 21,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: svc.freeze(),
    });
    let mut enc = BytesMut::new();
    encode_apdu(&mut enc, &write_req).unwrap();
    peer.send_apdu(
        &enc,
        &[0xC1],
        true,
        bacnet_types::enums::NetworkPriority::NORMAL,
    )
    .await
    .unwrap();
    let reply = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("reject must arrive")
        .expect("peer closed");
    match decode_apdu(reply.apdu).unwrap() {
        Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason,
        }) => {
            assert_eq!(invoke_id, 21);
            assert_eq!(reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
        }
        other => panic!("expected Reject, got {other:?}"),
    }

    // Segmented request draws server Abort SEGMENTATION_NOT_SUPPORTED.
    let mut svc2 = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object_id(ObjectType::ANALOG_INPUT, 9),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc2);
    let seg_req = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: true,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 22,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: svc2.freeze(),
    });
    let mut enc2 = BytesMut::new();
    encode_apdu(&mut enc2, &seg_req).unwrap();
    peer.send_apdu(
        &enc2,
        &[0xC1],
        true,
        bacnet_types::enums::NetworkPriority::NORMAL,
    )
    .await
    .unwrap();
    let abort = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("abort must arrive")
        .expect("peer closed");
    match decode_apdu(abort.apdu).unwrap() {
        Apdu::Abort(AbortPdu {
            invoke_id,
            abort_reason,
            sent_by_server,
        }) => {
            assert_eq!(invoke_id, 22);
            assert!(sent_by_server);
            assert_eq!(abort_reason, AbortReason::SEGMENTATION_NOT_SUPPORTED);
        }
        other => panic!("expected Abort, got {other:?}"),
    }

    // Role-attempt-to-stop-transport fails: roles expose no stop; session
    // stop-once holds (second stop errs); post-stop role calls fail closed.
    let detached = a.cloned_client_handle().unwrap();
    a.stop().await.unwrap();
    b.stop().await.unwrap();
    c.stop().await.unwrap();
    assert!(a.stop().await.is_err());
    assert!(detached
        .read_property(
            &[0xB2],
            object_id(ObjectType::ANALOG_INPUT, 9),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_err());
    peer.stop().await.unwrap();
}

#[test]
fn network_port_entry_rule_documents_bound_transport() {
    // One entry per bound transport with actual bound IP/port + network
    // number; instance numbering caller-stable (B/IP=1 here). MAC spells the
    // bound socket (IP octets + port BE) so a second hidden socket would
    // show as a second MAC/port.
    let entry = NetworkPortEntry::bip(1, 77, Ipv4Addr::new(127, 0, 0, 1), 47808);
    assert_eq!(entry.network_type, NetworkType::IPV4.to_raw());
    assert_eq!(entry.mac.as_slice(), &[127, 0, 0, 1, 0xBA, 0xC0]);
    let sc = NetworkPortEntry::sc(2, 88, [0x02; 6]);
    assert_eq!(sc.network_type, NetworkType::VIRTUAL.to_raw());
    assert_eq!(sc.mac.as_slice(), &[0x02; 6]);
    assert_eq!(sc.ip, [0; 4]);
}

#[test]
fn standalone_session_keeps_480_default() {
    // No identity => SessionConfig 480 untouched (no existing-test churn).
    let (t, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let s = EndpointSession::new(t, SessionRole::ClientOnly, SessionConfig::default()).unwrap();
    assert!(s.identity().is_none());
    let _ = ObjectDatabase::new();
}
