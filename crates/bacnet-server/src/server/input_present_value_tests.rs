use super::super::cov_notifications_tests::RecordingTransport;
use super::super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{EventState, Reliability};
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

type SentFrames = StdArc<StdMutex<Vec<(Bytes, MacAddr)>>>;

struct Fixture {
    server: BACnetServer<RecordingTransport>,
    sent: SentFrames,
    ai: ObjectIdentifier,
    bi: ObjectIdentifier,
    msi: ObjectIdentifier,
    av: ObjectIdentifier,
}

async fn fixture() -> Fixture {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, PropertyValue::Real(80.0)),
        (PropertyIdentifier::LOW_LIMIT, PropertyValue::Real(20.0)),
        (PropertyIdentifier::DEADBAND, PropertyValue::Real(2.0)),
        (
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0xC0],
            },
        ),
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x80],
            },
        ),
    ] {
        ai.write_property(property, None, value, None).unwrap();
    }

    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let ids = (
        ai.object_identifier(),
        bi.object_identifier(),
        msi.object_identifier(),
        av.object_identifier(),
    );
    let device = DeviceObject::new(DeviceConfig {
        instance: 100,
        name: "Input-local-write-test-device".into(),
        ..DeviceConfig::default()
    })
    .unwrap();

    let mut db = clocked_test_database();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(bi)).unwrap();
    db.add(Box::new(msi)).unwrap();
    db.add(Box::new(av)).unwrap();
    let server = BACnetServer::generic_builder()
        .transport(RecordingTransport::new(StdArc::clone(&sent)))
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    Fixture {
        server,
        sent,
        ai: ids.0,
        bi: ids.1,
        msi: ids.2,
        av: ids.3,
    }
}

async fn read(
    server: &BACnetServer<RecordingTransport>,
    oid: &ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(oid)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn assert_protocol_error(error: Error, class: ErrorClass, code: ErrorCode) {
    match error {
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } => {
            assert_eq!(actual_class, class.to_raw() as u32);
            assert_eq!(actual_code, code.to_raw() as u32);
        }
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

async fn subscribe(server: &BACnetServer<RecordingTransport>, oid: ObjectIdentifier) {
    server.cov_table.write().await.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[
            127,
            0,
            0,
            1,
            0xBA,
            oid.object_type().to_raw() as u8,
        ]),
        subscriber_network: None,
        subscriber_process_identifier: oid.object_type().to_raw(),
        monitored_object_identifier: oid,
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    });
}

async fn object_state(
    server: &BACnetServer<RecordingTransport>,
    oid: &ObjectIdentifier,
) -> (PropertyValue, PropertyValue) {
    (
        read(server, oid, PropertyIdentifier::PRESENT_VALUE).await,
        read(server, oid, PropertyIdentifier::RELIABILITY).await,
    )
}

async fn assert_rejected_without_mutation(
    fixture: &Fixture,
    oid: &ObjectIdentifier,
    value: PropertyValue,
    code: ErrorCode,
) {
    let before = object_state(&fixture.server, oid).await;
    let sent_before = fixture.sent.lock().unwrap().len();
    let error = fixture
        .server
        .set_present_value_local(oid, value)
        .await
        .expect_err("invalid input value must be rejected");
    assert_protocol_error(error, ErrorClass::PROPERTY, code);
    assert_eq!(object_state(&fixture.server, oid).await, before);
    assert_eq!(fixture.sent.lock().unwrap().len(), sent_before);
}

#[tokio::test]
async fn application_path_updates_only_supported_inputs_while_in_service() {
    let mut fixture = fixture().await;
    for (oid, value) in [
        (fixture.ai, PropertyValue::Real(21.5)),
        (fixture.bi, PropertyValue::Enumerated(1)),
        (fixture.msi, PropertyValue::Unsigned(2)),
    ] {
        fixture
            .server
            .set_present_value_local(&oid, value.clone())
            .await
            .unwrap();
        assert_eq!(
            read(&fixture.server, &oid, PropertyIdentifier::PRESENT_VALUE).await,
            value
        );
        assert_eq!(
            read(&fixture.server, &oid, PropertyIdentifier::OUT_OF_SERVICE).await,
            PropertyValue::Boolean(false)
        );
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn network_and_application_routes_preserve_input_simulation_ownership() {
    let mut fixture = fixture().await;
    subscribe(&fixture.server, fixture.ai).await;

    let error = fixture
        .server
        .write_local(
            &fixture.ai,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(40.0),
            None,
        )
        .await
        .expect_err("network-equivalent in-service write must be denied");
    assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(0.0)
    );
    assert!(fixture.sent.lock().unwrap().is_empty());

    fixture
        .server
        .write_local(
            &fixture.ai,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .await
        .unwrap();
    fixture
        .server
        .write_local(
            &fixture.ai,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(72.0),
            None,
        )
        .await
        .unwrap();
    let reliability = read(
        &fixture.server,
        &fixture.ai,
        PropertyIdentifier::RELIABILITY,
    )
    .await;
    let sent_before_rejection = fixture.sent.lock().unwrap().len();

    let error = fixture
        .server
        .set_present_value_local(&fixture.ai, PropertyValue::Real(19.0))
        .await
        .expect_err("application must not replace an OOS simulation");
    assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(72.0)
    );
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::RELIABILITY
        )
        .await,
        reliability
    );
    assert_eq!(
        fixture.sent.lock().unwrap().len(),
        sent_before_rejection,
        "rejected application update must not enter the COV path"
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn server_path_rejects_invalid_input_values_atomically() {
    let mut fixture = fixture().await;
    for oid in [fixture.ai, fixture.bi, fixture.msi] {
        subscribe(&fixture.server, oid).await;
    }
    for (oid, value) in [
        (fixture.ai, PropertyValue::Real(21.5)),
        (fixture.bi, PropertyValue::Enumerated(1)),
        (fixture.msi, PropertyValue::Unsigned(2)),
    ] {
        fixture
            .server
            .set_present_value_local(&oid, value)
            .await
            .unwrap();
    }
    fixture.sent.lock().unwrap().clear();

    for (oid, value, code) in [
        (
            fixture.ai,
            PropertyValue::Enumerated(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            fixture.ai,
            PropertyValue::Real(f32::NAN),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            fixture.bi,
            PropertyValue::Boolean(false),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            fixture.bi,
            PropertyValue::Enumerated(2),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            fixture.msi,
            PropertyValue::Enumerated(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            fixture.msi,
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            fixture.msi,
            PropertyValue::Unsigned(4),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_rejected_without_mutation(&fixture, &oid, value, code).await;
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn unknown_and_unsupported_objects_fail_before_side_effects() {
    let mut fixture = fixture().await;
    let unknown = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    subscribe(&fixture.server, fixture.av).await;
    subscribe(&fixture.server, unknown).await;

    let error = fixture
        .server
        .set_present_value_local(&unknown, PropertyValue::Real(1.0))
        .await
        .expect_err("unknown object must fail");
    assert_protocol_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    let error = fixture
        .server
        .set_present_value_local(&fixture.av, PropertyValue::Real(42.0))
        .await
        .expect_err("commandable object must not use the privileged input path");
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(
        read(
            &fixture.server,
            &fixture.av,
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        read(
            &fixture.server,
            &fixture.av,
            PropertyIdentifier::RELIABILITY
        )
        .await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert!(fixture.sent.lock().unwrap().is_empty());

    fixture
        .server
        .write_local(
            &fixture.av,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.0),
            Some(8),
        )
        .await
        .expect("generic local route must retain commandable-object behavior");
    assert_eq!(
        read(
            &fixture.server,
            &fixture.av,
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(42.0)
    );
    assert_eq!(fixture.sent.lock().unwrap().len(), 1);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn successful_input_update_runs_existing_event_and_cov_pipeline() {
    let mut fixture = fixture().await;
    subscribe(&fixture.server, fixture.ai).await;

    fixture
        .server
        .set_present_value_local(&fixture.ai, PropertyValue::Real(81.0))
        .await
        .unwrap();
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::EVENT_STATE
        )
        .await,
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "the existing intrinsic-event evaluator must observe the local update"
    );
    let frame = fixture.sent.lock().unwrap()[0].0.clone();
    let npdu = decode_npdu(frame).unwrap();
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected unconfirmed COV notification");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    let notification = COVNotificationRequest::decode(&request.service_request).unwrap();
    assert_eq!(notification.monitored_object_identifier, fixture.ai);
    assert_eq!(fixture.sent.lock().unwrap().len(), 1);

    let error = fixture
        .server
        .set_present_value_local(&fixture.ai, PropertyValue::Real(f32::NAN))
        .await
        .expect_err("rejected update must stop before post-write processing");
    assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(81.0)
    );
    assert_eq!(
        read(
            &fixture.server,
            &fixture.ai,
            PropertyIdentifier::EVENT_STATE
        )
        .await,
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
    assert_eq!(
        fixture.sent.lock().unwrap().len(),
        1,
        "rejected update must not emit COV"
    );
    fixture.server.stop().await.unwrap();
}
