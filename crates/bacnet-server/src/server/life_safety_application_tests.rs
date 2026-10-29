//! The application's runtime route to a stored Life Safety Point's or Zone's
//! Present_Value and Tracking_Value (#1123).
//!
//! `set_present_value_local` and `set_tracking_value_local` reach the boxed
//! object through its internal hooks and notify through the Life Safety COV
//! snapshots once the database lock is released: Present_Value reaches
//! SubscribeCOV and Present_Value property subscribers, Tracking_Value only
//! its property subscribers. While Out_Of_Service is TRUE an application
//! Tracking_Value waits for the return to service (#1108).
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_types::enums::{EngineeringUnits, LifeSafetyState, ObjectType};
use bacnet_types::error::RemoteError;
use std::collections::BTreeMap;

const TV: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;

fn point() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn zone() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap()
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()))
            .unwrap();
        db.add(Box::new(LifeSafetyZoneObject::new(1, "LSZ-1").unwrap()))
            .unwrap();
        db.add(Box::new(
            AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await
}

fn state(state: LifeSafetyState) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

/// A reported state property and its value bytes.
fn row(property: PropertyIdentifier, value: LifeSafetyState) -> (PropertyIdentifier, Vec<u8>) {
    (property, encoded(&state(value)))
}

/// Status_Flags and its value bytes, with at most OUT_OF_SERVICE set.
fn flags(out_of_service: bool) -> (PropertyIdentifier, Vec<u8>) {
    let data = vec![if out_of_service { 0x10 } else { 0x00 }];
    (
        SF,
        encoded(&PropertyValue::BitString {
            unused_bits: 4,
            data,
        }),
    )
}

type Rows = Vec<(PropertyIdentifier, Vec<u8>)>;

fn values(notification: &COVNotificationRequest) -> (ObjectIdentifier, Rows) {
    (
        notification.monitored_object_identifier,
        notification
            .list_of_values
            .iter()
            .map(|value| (value.property_identifier, value.value.clone()))
            .collect(),
    )
}

/// SubscribeCOV on `object` for `process`, or SubscribeCOVProperty on one of
/// its properties.
async fn subscribe(
    h: &mut Harness,
    process: u32,
    object: ObjectIdentifier,
    property: Option<PropertyIdentifier>,
) {
    let mut body = BytesMut::new();
    let service = match property {
        None => {
            SubscribeCOVRequest {
                subscriber_process_identifier: process,
                monitored_object_identifier: object,
                issue_confirmed_notifications: Some(false),
                lifetime: Some(300),
            }
            .encode(&mut body)
            .unwrap();
            ConfirmedServiceChoice::SUBSCRIBE_COV
        }
        Some(property) => {
            SubscribeCOVPropertyRequest {
                subscriber_process_identifier: process,
                monitored_object_identifier: object,
                issue_confirmed_notifications: Some(false),
                lifetime: Some(300),
                monitored_property_identifier: property,
                monitored_property_array_index: None,
                cov_increment: None,
            }
            .encode(&mut body)
            .unwrap();
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY
        }
    };
    h.request(service, body).await;
    // The initial report follows the ACK; tests start from the state after it.
    h.cov_notification().await;
}

/// The next `count` notifications, keyed by subscriber process.
async fn notifications(h: &Harness, count: usize) -> BTreeMap<u32, (ObjectIdentifier, Rows)> {
    let mut by_process = BTreeMap::new();
    for _ in 0..count {
        let notification = h.cov_notification().await;
        let previous = by_process.insert(
            notification.subscriber_process_identifier,
            values(&notification),
        );
        assert!(previous.is_none(), "one notification per subscription");
    }
    by_process
}

async fn read(
    h: &Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

async fn set_out_of_service(h: &Harness, object: ObjectIdentifier, out_of_service: bool) {
    h.server
        .write_local(
            &object,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
            if c == class && e == code),
        "expected {class:?}/{code:?}, got {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn life_safety_runtime_values_notify_their_cov_subscribers() {
    let mut h = start().await;
    for object in [point(), zone()] {
        // 1: SubscribeCOV, 2: Present_Value property, 3: Tracking_Value property.
        subscribe(&mut h, 1, object, None).await;
        subscribe(&mut h, 2, object, Some(PV)).await;
        subscribe(&mut h, 3, object, Some(TV)).await;

        h.server
            .set_present_value_local(&object, state(LifeSafetyState::ALARM))
            .await
            .unwrap();
        let reported = vec![row(PV, LifeSafetyState::ALARM), flags(false)];
        assert_eq!(
            notifications(&h, 2).await,
            BTreeMap::from([(1, (object, reported.clone())), (2, (object, reported))])
        );
        h.no_notification().await;

        // A latching application keeps Present_Value while Tracking_Value
        // follows the live state; only the Tracking_Value subscriber hears.
        h.server
            .set_tracking_value_local(&object, state(LifeSafetyState::PRE_ALARM))
            .await
            .unwrap();
        assert_eq!(
            notifications(&h, 1).await,
            BTreeMap::from([(
                3,
                (
                    object,
                    vec![row(TV, LifeSafetyState::PRE_ALARM), flags(false)]
                )
            )])
        );
        h.no_notification().await;
        assert_eq!(read(&h, object, PV).await, state(LifeSafetyState::ALARM));

        // Repeating either value changes nothing, so nobody is notified.
        h.server
            .set_present_value_local(&object, state(LifeSafetyState::ALARM))
            .await
            .unwrap();
        h.server
            .set_tracking_value_local(&object, state(LifeSafetyState::PRE_ALARM))
            .await
            .unwrap();
        h.no_notification().await;
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn life_safety_runtime_values_refuse_bad_values_and_targets() {
    let mut h = start().await;
    subscribe(&mut h, 1, point(), None).await;
    subscribe(&mut h, 3, point(), Some(TV)).await;

    // 35..=255 are reserved for ASHRAE; past 65535 is outside the datatype.
    let refused = [
        (PropertyValue::Enumerated(35), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Enumerated(65_536),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Unsigned(2), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
    ];
    for (value, code) in refused {
        let error = h
            .server
            .set_present_value_local(&point(), value.clone())
            .await
            .unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, code);
        let error = h
            .server
            .set_tracking_value_local(&point(), value)
            .await
            .unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, code);
    }
    assert_eq!(read(&h, point(), PV).await, state(LifeSafetyState::QUIET));
    assert_eq!(read(&h, point(), TV).await, state(LifeSafetyState::QUIET));

    let unknown = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 9).unwrap();
    let error = h
        .server
        .set_present_value_local(&unknown, state(LifeSafetyState::ALARM))
        .await
        .unwrap_err();
    assert_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    let error = h
        .server
        .set_tracking_value_local(&unknown, state(LifeSafetyState::ALARM))
        .await
        .unwrap_err();
    assert_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    // Neither an Analog Value nor an Analog Input, which does take an
    // application Present_Value, has a Tracking_Value to set.
    for other in [av1(), ai1()] {
        let error = h
            .server
            .set_tracking_value_local(&other, state(LifeSafetyState::ALARM))
            .await
            .unwrap_err();
        assert_error(
            error,
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
    }
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn out_of_service_holds_the_application_tracking_value_until_return_to_service() {
    let mut h = start().await;
    for object in [point(), zone()] {
        subscribe(&mut h, 1, object, None).await;
        subscribe(&mut h, 3, object, Some(TV)).await;

        set_out_of_service(&h, object, true).await;
        assert_eq!(
            notifications(&h, 2).await,
            BTreeMap::from([
                (
                    1,
                    (object, vec![row(PV, LifeSafetyState::QUIET), flags(true)])
                ),
                (
                    3,
                    (object, vec![row(TV, LifeSafetyState::QUIET), flags(true)])
                ),
            ])
        );

        // The application's value is set aside: nothing served changes.
        h.server
            .set_tracking_value_local(&object, state(LifeSafetyState::ALARM))
            .await
            .unwrap();
        h.no_notification().await;
        assert_eq!(read(&h, object, TV).await, state(LifeSafetyState::QUIET));

        // A client simulation is served and notified; a later application
        // value still waits behind it.
        h.server
            .write_local(
                &object,
                TV,
                None,
                state(LifeSafetyState::TAMPER),
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
        assert_eq!(
            notifications(&h, 1).await,
            BTreeMap::from([(
                3,
                (object, vec![row(TV, LifeSafetyState::TAMPER), flags(true)])
            )])
        );
        h.server
            .set_tracking_value_local(&object, state(LifeSafetyState::PRE_ALARM))
            .await
            .unwrap();
        h.no_notification().await;
        assert_eq!(read(&h, object, TV).await, state(LifeSafetyState::TAMPER));

        // Present_Value isn't decoupled: served and notified at once.
        h.server
            .set_present_value_local(&object, state(LifeSafetyState::ALARM))
            .await
            .unwrap();
        assert_eq!(
            notifications(&h, 1).await,
            BTreeMap::from([(
                1,
                (object, vec![row(PV, LifeSafetyState::ALARM), flags(true)])
            )])
        );

        // The return to service serves the latest application value.
        set_out_of_service(&h, object, false).await;
        assert_eq!(
            notifications(&h, 2).await,
            BTreeMap::from([
                (
                    1,
                    (object, vec![row(PV, LifeSafetyState::ALARM), flags(false)])
                ),
                (
                    3,
                    (
                        object,
                        vec![row(TV, LifeSafetyState::PRE_ALARM), flags(false)]
                    )
                ),
            ])
        );
        h.no_notification().await;
        assert_eq!(
            read(&h, object, TV).await,
            state(LifeSafetyState::PRE_ALARM)
        );
    }
    h.server.stop().await.unwrap();
}
