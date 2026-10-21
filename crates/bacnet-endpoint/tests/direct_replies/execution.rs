use super::*;
use bacnet_encoding::{apdu::ConfirmedRequest, primitives::encode_app_character_string};
use bacnet_objects::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_services::{
    read_property::{ReadPropertyACK, ReadPropertyRequest},
    write_property::WritePropertyRequest,
};
use bacnet_types::{
    enums::{AbortReason, ConfirmedServiceChoice, ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use bytes::{Bytes, BytesMut};
use std::sync::{Arc, Mutex};

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap()
}
fn write(text: &str, invoke: u8) -> Apdu {
    let mut value = BytesMut::new();
    encode_app_character_string(&mut value, text).unwrap();
    let mut service = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut service)
    .unwrap();
    confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        invoke,
        service.freeze(),
    )
}
fn read_description(invoke: u8) -> Apdu {
    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
    }
    .encode(&mut service);
    confirmed(
        ConfirmedServiceChoice::READ_PROPERTY,
        invoke,
        service.freeze(),
    )
}
async fn assert_description(f: &mut Fixture, peer: &Peer, text: &str, invoke: u8) {
    let Apdu::ComplexAck(ack) = f.response(peer).await.1 else {
        panic!("readback ComplexACK")
    };
    assert_eq!(ack.invoke_id, invoke);
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    let mut expected = BytesMut::new();
    encode_app_character_string(&mut expected, text).unwrap();
    assert_eq!(ack.property_value, expected);
}
#[tokio::test]
async fn retired_and_missing_reply_authority_do_not_revoke_admitted_device_execution() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(123, 0).expect("valid Device identity"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let allowed = Arc::new(Mutex::new(None));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let auth_allowed = allowed.clone();
    let auth_observed = observed.clone();
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .with_database(db)
        .with_device_writes(Arc::new(move |context| {
            let identity = context.provenance.direct_sc_identity().unwrap();
            auth_observed.lock().unwrap().push(identity);
            *auth_allowed.lock().unwrap() == Some(identity)
        }));
    session.start().await.unwrap();
    let mut a = f.peer(ca.tls("A"), None).await;
    let held = f
        .capture(&mut a, &write("retired A committed", 30), None)
        .await;
    let identity_a = held.provenance.direct_sc_identity().unwrap();
    *allowed.lock().unwrap() = Some(identity_a);
    let mut missing = f
        .capture(&mut a, &write("missing route committed", 31), None)
        .await;
    missing.direct_response = None;
    let mut b = f.peer(ca.tls("B"), None).await;
    let read = f.capture(&mut b, &read_description(32), None).await;
    f.feed(held).await;
    f.feed(read).await;
    assert_description(&mut f, &b, "retired A committed", 32).await;
    f.feed(missing).await;
    let read = f.capture(&mut b, &read_description(33), None).await;
    f.feed(read).await;
    assert_description(&mut f, &b, "missing route committed", 33).await;
    let denied = f.capture(&mut b, &write("must not commit", 34), None).await;
    let identity_b = denied.provenance.direct_sc_identity().unwrap();
    f.feed(denied).await;
    assert!(matches!(f.response(&b).await.1, Apdu::Error(e) if e.invoke_id == 34));
    assert_eq!(
        *observed.lock().unwrap(),
        [identity_a, identity_a, identity_b]
    );
    let read = f.capture(&mut b, &read_description(35), None).await;
    f.feed(read).await;
    assert_description(&mut f, &b, "missing route committed", 35).await;
    *allowed.lock().unwrap() = Some(identity_b);
    let live = f
        .capture(&mut b, &write("current B committed", 36), None)
        .await;
    f.feed(live).await;
    assert!(matches!(f.response(&b).await.1, Apdu::SimpleAck(a) if a.invoke_id == 36));
    let read = f.capture(&mut b, &read_description(37), None).await;
    f.feed(read).await;
    assert_description(&mut f, &b, "current B committed", 37).await;
    f.barrier(&mut b, 38).await;
    assert_eq!(session.policy_counters().await.responder_declined, 2);
    session.stop().await.unwrap();
    f.listener.stop().await;
}
#[tokio::test]
async fn endpoint_segmented_request_abort_and_read_error_use_original_socket() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session =
        EndpointSession::new(port, SessionRole::Both, SessionConfig::default()).unwrap();
    session.start().await.unwrap();
    let mut peer = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut peer, &read_name(40), None).await;
    f.feed(admitted).await;
    assert!(matches!(f.response(&peer).await.1, Apdu::Error(e) if e.invoke_id == 40));
    let Apdu::ConfirmedRequest(request) =
        confirmed(ConfirmedServiceChoice::READ_PROPERTY, 41, Bytes::new())
    else {
        unreachable!()
    };
    let request = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: true,
        more_follows: true,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        ..request
    });
    let admitted = f.capture(&mut peer, &request, None).await;
    f.feed(admitted).await;
    assert!(
        matches!(f.response(&peer).await.1, Apdu::Abort(a) if a.invoke_id == 41 && a.sent_by_server && a.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    session.stop().await.unwrap();
    f.listener.stop().await;
}
