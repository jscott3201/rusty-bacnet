//! The application route for a stored Loop's Controlled_Variable_Value (#1063).
//!
//! `set_controlled_variable_value_local` takes the measurement from the
//! application that runs the loop's algorithm, through the object's internal
//! hook rather than a downcast. Its change reaches a SubscribeCOVProperty on
//! the property. A SubscribeCOV on the Loop reports the value without being
//! triggered by it (Table 13-1), so it only shows up in the next report.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType};

const CVV: PropertyIdentifier = PropertyIdentifier::CONTROLLED_VARIABLE_VALUE;
const SETPOINT: PropertyIdentifier = PropertyIdentifier::SETPOINT;
const OUT_OF_SERVICE: u8 = 0x10;

fn loop1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LOOP, 1).unwrap()
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await
}

/// `(property, value bytes)` of a notification about LOOP-1, in wire order.
fn values(notification: &COVNotificationRequest) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    assert_eq!(notification.monitored_object_identifier, loop1());
    notification
        .list_of_values
        .iter()
        .map(|value| (value.property_identifier, value.value.clone()))
        .collect()
}

fn flags(bits: u8) -> Vec<u8> {
    vec![0x82, 0x04, bits]
}

async fn subscribe_controlled_variable_value(h: &mut Harness) {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1063,
        monitored_object_identifier: loop1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: CVV,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
}

async fn subscribe_loop(h: &mut Harness) {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1062,
        monitored_object_identifier: loop1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
}

async fn loop_value(h: &Harness, property: PropertyIdentifier) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&loop1())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

async fn set_out_of_service(h: &Harness, value: bool) {
    h.server
        .write_local(
            &loop1(),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(value),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class: c, code: e }
            if c == class && e == code),
        "expected {class:?}/{code:?}, got {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn controlled_variable_value_local_notifies_a_property_subscription() {
    let mut h = start().await;
    subscribe_controlled_variable_value(&mut h).await;
    assert_eq!(
        values(&h.cov_notification().await),
        [(CVV, real(0.0)), (SF, flags(0))],
        "initial report"
    );

    for measured in [21.5, 22.0] {
        h.server
            .set_controlled_variable_value_local(&loop1(), PropertyValue::Real(measured))
            .await
            .unwrap();
        assert_eq!(loop_value(&h, CVV).await, PropertyValue::Real(measured));
        assert_eq!(
            values(&h.cov_notification().await),
            [(CVV, real(measured)), (SF, flags(0))]
        );
    }

    // Out_Of_Service doesn't decouple the measurement: it is still taken and
    // still reported.
    set_out_of_service(&h, true).await;
    assert_eq!(
        values(&h.cov_notification().await),
        [(CVV, real(22.0)), (SF, flags(OUT_OF_SERVICE))]
    );
    h.server
        .set_controlled_variable_value_local(&loop1(), PropertyValue::Real(19.25))
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await),
        [(CVV, real(19.25)), (SF, flags(OUT_OF_SERVICE))]
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn controlled_variable_value_local_rides_the_next_subscribe_cov_report() {
    let mut h = start().await;
    subscribe_loop(&mut h).await;
    let report = |pv: f32, controlled: f32| {
        vec![
            (PV, real(pv)),
            (SF, flags(0)),
            (SETPOINT, real(0.0)),
            (CVV, real(controlled)),
        ]
    };
    assert_eq!(values(&h.cov_notification().await), report(0.0, 0.0));

    // A change of the measurement alone triggers no SubscribeCOV report...
    h.server
        .set_controlled_variable_value_local(&loop1(), PropertyValue::Real(20.5))
        .await
        .unwrap();
    h.no_notification().await;
    // ...and the next report, here for a new output, carries it.
    h.server
        .set_present_value_local(&loop1(), PropertyValue::Real(35.0))
        .await
        .unwrap();
    assert_eq!(values(&h.cov_notification().await), report(35.0, 20.5));
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn controlled_variable_value_local_refuses_bad_values_and_targets() {
    let mut h = start().await;
    h.server
        .set_controlled_variable_value_local(&loop1(), PropertyValue::Real(18.0))
        .await
        .unwrap();
    subscribe_controlled_variable_value(&mut h).await;
    h.cov_notification().await;

    for (value, code) in [
        (PropertyValue::Unsigned(19), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::NEG_INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        let error = h
            .server
            .set_controlled_variable_value_local(&loop1(), value)
            .await
            .unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, code);
    }
    assert_eq!(loop_value(&h, CVV).await, PropertyValue::Real(18.0));

    let unknown = ObjectIdentifier::new(ObjectType::LOOP, 9).unwrap();
    let error = h
        .server
        .set_controlled_variable_value_local(&unknown, PropertyValue::Real(1.0))
        .await
        .unwrap_err();
    assert_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    let error = h
        .server
        .set_controlled_variable_value_local(&av1(), PropertyValue::Real(1.0))
        .await
        .unwrap_err();
    assert_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    h.no_notification().await;

    // The network route stays closed.
    let mut db = h.server.database().write().await;
    let error = db
        .get_mut(&loop1())
        .unwrap()
        .write_property(CVV, None, PropertyValue::Real(5.0), None)
        .unwrap_err();
    assert_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    drop(db);
    h.server.stop().await.unwrap();
}
