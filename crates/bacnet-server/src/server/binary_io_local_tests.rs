//! The application's routes to a running Binary Input's and Binary Output's
//! physical side and Polarity (#2580).

use super::super::cov_notifications_tests::recording_transport;
use super::super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{EngineeringUnits, EventState, Polarity};
use bacnet_types::error::RemoteError;

const INACTIVE: PropertyValue = PropertyValue::Enumerated(0);
const ACTIVE: PropertyValue = PropertyValue::Enumerated(1);

struct Fixture {
    server: BACnetServer<TestTransport>,
    sent: SendLog,
    bi: ObjectIdentifier,
    bo: ObjectIdentifier,
    ai: ObjectIdentifier,
}

async fn fixture() -> Fixture {
    let (transport, sent) = recording_transport();
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let ids = (
        bi.object_identifier(),
        bo.object_identifier(),
        ai.object_identifier(),
    );
    let identity = bacnet_objects::DeviceIdentity::new(100, 0).unwrap();
    let device = DeviceObject::new(DeviceConfig {
        identity: identity.clone().with_name("Polarity-test-device"),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(bi)).unwrap();
    db.add(Box::new(bo)).unwrap();
    db.add(Box::new(ai)).unwrap();
    let server = BACnetServer::generic_builder()
        .device(identity)
        .transport(transport)
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    Fixture {
        server,
        sent,
        bi: ids.0,
        bo: ids.1,
        ai: ids.2,
    }
}

async fn read(
    fixture: &Fixture,
    oid: &ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    fixture
        .server
        .read_local(oid, property, None)
        .await
        .unwrap()
}

async fn write(
    fixture: &Fixture,
    oid: &ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    fixture
        .server
        .write_local(
            oid,
            property,
            None,
            value,
            priority,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

fn assert_protocol_error(
    result: Result<impl core::fmt::Debug, Error>,
    class: ErrorClass,
    code: ErrorCode,
) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: actual_class,
            code: actual_code,
        })) => assert_eq!((actual_class, actual_code), (class, code)),
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

/// A whole-object SubscribeCOV on `oid`, so a notification carries
/// Present_Value and Status_Flags.
async fn subscribe(fixture: &Fixture, oid: ObjectIdentifier) {
    fixture
        .server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
}

/// The Present_Value the `index`th frame sent, a COV notification about
/// `oid`, carries, encoded.
fn notified_present_value(sent: &SendLog, index: usize, oid: ObjectIdentifier) -> Vec<u8> {
    let frame = sent.frame(index).npdu;
    let npdu = decode_npdu(frame).unwrap();
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected an unconfirmed COV notification");
    };
    let notification = COVNotificationRequest::decode(&request.service_request).unwrap();
    assert_eq!(notification.monitored_object_identifier, oid);
    notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .expect("Present_Value reported")
        .value
        .clone()
}

/// An encoded Enumerated of one octet.
fn encoded_enumerated(value: u8) -> Vec<u8> {
    vec![0x91, value]
}

/// The physical input reaches the event algorithm and COV subscribers as
/// the logical Present_Value: an OFF input under REVERSE is ACTIVE, the
/// default Alarm_Value.
#[tokio::test]
async fn physical_input_reports_the_logical_state() {
    let mut fixture = fixture().await;
    fixture
        .server
        .set_polarity_local(&fixture.bi, Polarity::REVERSE)
        .await
        .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::POLARITY).await,
        PropertyValue::Enumerated(Polarity::REVERSE.to_raw())
    );
    subscribe(&fixture, fixture.bi).await;
    fixture.sent.clear();

    fixture
        .server
        .set_physical_input_local(&fixture.bi, false)
        .await
        .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::PRESENT_VALUE).await,
        ACTIVE
    );
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::EVENT_STATE).await,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw())
    );
    assert!(!fixture.sent.is_empty(), "a COV notification is sent");
    assert_eq!(
        notified_present_value(&fixture.sent, 0, fixture.bi),
        encoded_enumerated(1)
    );
    fixture.server.stop().await.unwrap();
}

/// In service a Polarity change derives Present_Value from the steady
/// input again, and subscribers hear of it.
#[tokio::test]
async fn polarity_change_derives_present_value_again() {
    let mut fixture = fixture().await;
    fixture
        .server
        .set_physical_input_local(&fixture.bi, true)
        .await
        .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::PRESENT_VALUE).await,
        ACTIVE
    );
    subscribe(&fixture, fixture.bi).await;
    fixture.sent.clear();

    fixture
        .server
        .set_polarity_local(&fixture.bi, Polarity::REVERSE)
        .await
        .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::PRESENT_VALUE).await,
        INACTIVE
    );
    assert!(!fixture.sent.is_empty(), "a COV notification is sent");
    assert_eq!(
        notified_present_value(&fixture.sent, 0, fixture.bi),
        encoded_enumerated(0)
    );
    fixture.server.stop().await.unwrap();
}

/// Out of service the input is kept but Present_Value stays the client's;
/// the return to service derives it from the latest input.
#[tokio::test]
async fn physical_input_is_kept_out_of_service() {
    let mut fixture = fixture().await;
    let oos = PropertyIdentifier::OUT_OF_SERVICE;
    write(
        &fixture,
        &fixture.bi,
        oos,
        PropertyValue::Boolean(true),
        None,
    )
    .await
    .unwrap();
    fixture
        .server
        .set_physical_input_local(&fixture.bi, true)
        .await
        .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::PRESENT_VALUE).await,
        INACTIVE
    );
    write(
        &fixture,
        &fixture.bi,
        oos,
        PropertyValue::Boolean(false),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::PRESENT_VALUE).await,
        ACTIVE
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn physical_output_follows_commands_and_polarity() {
    let mut fixture = fixture().await;
    let bo = fixture.bo;
    assert!(!fixture.server.physical_output_local(&bo).await.unwrap());
    write(
        &fixture,
        &bo,
        PropertyIdentifier::PRESENT_VALUE,
        ACTIVE,
        Some(8),
    )
    .await
    .unwrap();
    assert!(fixture.server.physical_output_local(&bo).await.unwrap());

    fixture
        .server
        .set_polarity_local(&bo, Polarity::REVERSE)
        .await
        .unwrap();
    assert!(!fixture.server.physical_output_local(&bo).await.unwrap());
    assert_eq!(
        read(&fixture, &bo, PropertyIdentifier::PRESENT_VALUE).await,
        ACTIVE
    );
    // Read-only on the network, and to a network-equivalent local write.
    assert_protocol_error(
        write(
            &fixture,
            &bo,
            PropertyIdentifier::POLARITY,
            PropertyValue::Enumerated(Polarity::NORMAL.to_raw()),
            None,
        )
        .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn binary_routes_refuse_other_objects_and_values() {
    let mut fixture = fixture().await;
    let unknown = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 9).unwrap();
    let server = &fixture.server;
    assert_protocol_error(
        server.physical_output_local(&unknown).await,
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    );
    assert_protocol_error(
        server.physical_output_local(&fixture.bi).await,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_protocol_error(
        server.set_physical_input_local(&fixture.bo, true).await,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_protocol_error(
        server
            .set_polarity_local(&fixture.ai, Polarity::REVERSE)
            .await,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_protocol_error(
        server.set_polarity_local(&unknown, Polarity::REVERSE).await,
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    );
    assert_protocol_error(
        server
            .set_polarity_local(&fixture.bi, Polarity::from_raw(2))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        read(&fixture, &fixture.bi, PropertyIdentifier::POLARITY).await,
        PropertyValue::Enumerated(Polarity::NORMAL.to_raw())
    );
    assert!(fixture.sent.is_empty(), "a refusal reports nothing");
    fixture.server.stop().await.unwrap();
}
