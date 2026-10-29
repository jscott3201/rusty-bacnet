//! Loop COV reporting over the wire (#978, #985).
//!
//! #978: Loop used to report a Status_Flags fixed at construction, so a
//! subscriber never heard of a fault or of Out_Of_Service. Each Out_Of_Service
//! or Reliability write below changes only Status_Flags and must produce one
//! notification carrying the new flags; a Setpoint write must produce none.
//!
//! #985: Table 13-1 has a Loop notification carry Setpoint and
//! Controlled_Variable_Value after Present_Value and Status_Flags, and fire
//! only when Present_Value moves by COV_Increment or Status_Flags changes.
//! Table 12-20 makes Present_Value writable while Out_Of_Service is TRUE.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{EngineeringUnits, ObjectType, Reliability};
use bacnet_types::error::RemoteError;

const FAULT: u8 = 0x40;
const OUT_OF_SERVICE: u8 = 0x10;
const SETPOINT: PropertyIdentifier = PropertyIdentifier::SETPOINT;
const CVV: PropertyIdentifier = PropertyIdentifier::CONTROLLED_VARIABLE_VALUE;

fn loop1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LOOP, 1).unwrap()
}

/// The Status_Flags bits of a notification's application-tagged BIT STRING.
fn cov_flags(notification: &COVNotificationRequest) -> u8 {
    assert_eq!(notification.monitored_object_identifier, loop1());
    let value = &notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == SF)
        .expect("Status_Flags reported with Present_Value")
        .value;
    assert_eq!(&value[..2], &[0x82, 0x04], "four-bit Status_Flags");
    value[2]
}

/// Encoded Status_Flags with `bits` already in the high nibble.
fn flags(bits: u8) -> Vec<u8> {
    vec![0x82, 0x04, bits]
}

/// `(property, value bytes)` of a Loop notification, in wire order.
fn values(notification: &COVNotificationRequest) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    assert_eq!(notification.monitored_object_identifier, loop1());
    notification
        .list_of_values
        .iter()
        .map(|value| {
            assert_eq!(value.property_array_index, None);
            assert_eq!(value.priority, None);
            (value.property_identifier, value.value.clone())
        })
        .collect()
}

/// The full Table 13-1 Loop report.
fn loop_report(
    pv: f32,
    status: u8,
    setpoint: f32,
    controlled: f32,
) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    vec![
        (PV, real(pv)),
        (SF, flags(status)),
        (SETPOINT, real(setpoint)),
        (CVV, real(controlled)),
    ]
}

async fn start(configure: impl FnOnce(&mut LoopObject)) -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        let mut object = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        configure(&mut object);
        db.add(Box::new(object)).unwrap();
    })
    .await
}

async fn subscribe_loop(h: &mut Harness) {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 978,
        monitored_object_identifier: loop1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
}

async fn write_loop(h: &mut Harness, property: PropertyIdentifier, value: PropertyValue) {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &value).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: loop1(),
        property_identifier: property,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
}

/// Wait for the SimpleACK answering the last request sent, and take it.
async fn wait_for_ack(h: &Harness) {
    assert_eq!(response(h).await, Ok(()), "the request must succeed");
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

#[tokio::test]
async fn loop_fault_and_out_of_service_notify_cov_subscribers() {
    let mut h = start(|_| {}).await;
    subscribe_loop(&mut h).await;
    assert_eq!(cov_flags(&h.cov_notification().await), 0, "initial report");

    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(cov_flags(&h.cov_notification().await), OUT_OF_SERVICE);

    write_loop(
        &mut h,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
    )
    .await;
    assert_eq!(
        cov_flags(&h.cov_notification().await),
        FAULT | OUT_OF_SERVICE,
        "a simulated fault must reach the subscriber"
    );

    // Returning to service restores the evaluated NO_FAULT_DETECTED.
    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .await;
    assert_eq!(cov_flags(&h.cov_notification().await), 0);

    // A write that leaves Present_Value and Status_Flags alone reports nothing.
    // Wait for its SimpleACK, then make a flags-changing write: the next
    // notification must be that one's, so a stray Setpoint report can't slip
    // past unseen however slowly it is sent.
    write_loop(&mut h, SETPOINT, PropertyValue::Real(21.0)).await;
    wait_for_ack(&h).await;
    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(
        cov_flags(&h.cov_notification().await),
        OUT_OF_SERVICE,
        "the Setpoint write must not have produced a notification"
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn loop_cov_notification_reports_setpoint_and_controlled_variable_value() {
    let mut h = start(|object| {
        object.set_present_value(40.0);
        object.set_controlled_variable_value(20.5);
        object
            .write_property(SETPOINT, None, PropertyValue::Real(21.0), None)
            .unwrap();
    })
    .await;
    subscribe_loop(&mut h).await;
    let initial = h.cov_notification().await;
    assert_eq!(initial.subscriber_process_identifier, 978);
    assert_eq!(values(&initial), loop_report(40.0, 0, 21.0, 20.5));

    // Setpoint is reported but is not a trigger: its change alone sends
    // nothing, and the next report carries its current value.
    write_loop(&mut h, SETPOINT, PropertyValue::Real(23.5)).await;
    wait_for_ack(&h).await;
    h.no_notification().await;
    h.server
        .set_present_value_local(&loop1(), PropertyValue::Real(41.0))
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(41.0, 0, 23.5, 20.5)
    );

    // A Status_Flags trigger carries the same four values.
    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(41.0, OUT_OF_SERVICE, 23.5, 20.5)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn loop_cov_increment_filters_present_value_reports() {
    let mut h = start(|_| {}).await;
    write_loop(
        &mut h,
        PropertyIdentifier::COV_INCREMENT,
        PropertyValue::Real(2.0),
    )
    .await;
    wait_for_ack(&h).await;
    subscribe_loop(&mut h).await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(0.0, 0, 0.0, 0.0)
    );

    // Each step moves the application's output; only a move of at least
    // COV_Increment from the last reported 0.0, then 2.0, sends a report.
    for (pv, reported) in [
        (1.0, false),
        (1.9, false),
        (2.0, true),
        (3.5, false),
        (0.5, false),
        (-0.1, true),
    ] {
        h.server
            .set_present_value_local(&loop1(), PropertyValue::Real(pv))
            .await
            .unwrap();
        if reported {
            assert_eq!(
                values(&h.cov_notification().await),
                loop_report(pv, 0, 0.0, 0.0),
                "a move to {pv} reaches the increment"
            );
        } else {
            h.no_notification().await;
        }
    }

    // A Status_Flags change reports even though Present_Value is unchanged.
    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(-0.1, OUT_OF_SERVICE, 0.0, 0.0)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn loop_present_value_write_property_follows_out_of_service() {
    let mut h = start(|_| {}).await;
    subscribe_loop(&mut h).await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(0.0, 0, 0.0, 0.0)
    );

    // In service the algorithm owns Present_Value.
    write_loop(&mut h, PV, PropertyValue::Real(50.0)).await;
    assert_eq!(response(&h).await, Err(ErrorCode::WRITE_ACCESS_DENIED));
    h.no_notification().await;
    assert_eq!(loop_value(&h, PV).await, PropertyValue::Real(0.0));

    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(0.0, OUT_OF_SERVICE, 0.0, 0.0)
    );

    // Out of service a peer simulates it, and the application is refused.
    write_loop(&mut h, PV, PropertyValue::Real(50.0)).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(50.0, OUT_OF_SERVICE, 0.0, 0.0)
    );
    let refused = h
        .server
        .set_present_value_local(&loop1(), PropertyValue::Real(10.0))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, Error::Remote(RemoteError::Error { code, .. })
            if code == ErrorCode::WRITE_ACCESS_DENIED),
        "{refused:?}"
    );
    h.no_notification().await;
    assert_eq!(loop_value(&h, PV).await, PropertyValue::Real(50.0));

    write_loop(
        &mut h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await),
        loop_report(50.0, 0, 0.0, 0.0)
    );
    write_loop(&mut h, PV, PropertyValue::Real(60.0)).await;
    assert_eq!(response(&h).await, Err(ErrorCode::WRITE_ACCESS_DENIED));
    h.no_notification().await;
    assert_eq!(loop_value(&h, PV).await, PropertyValue::Real(50.0));
    h.server.stop().await.unwrap();
}
