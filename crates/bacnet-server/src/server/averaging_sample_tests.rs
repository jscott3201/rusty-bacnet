//! The application route that feeds a stored Averaging object (#1083).
//!
//! `add_averaging_sample_local` takes each sample the application took,
//! through the object's internal hook rather than a downcast, and runs the
//! server's COV processing once the database lock is released. Table 13-1
//! has no Averaging row, so SubscribeCOV is refused; property subscriptions
//! follow Table 13-1a and carry no Status_Flags, since the object has none.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::averaging::AveragingObject;
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;
use std::collections::BTreeMap;

#[path = "averaging_window_tests.rs"]
mod window_tests;

#[path = "averaging_sampling_tests.rs"]
mod sampling_tests;

const MIN: PropertyIdentifier = PropertyIdentifier::MINIMUM_VALUE;
const MAX: PropertyIdentifier = PropertyIdentifier::MAXIMUM_VALUE;
const AVG: PropertyIdentifier = PropertyIdentifier::AVERAGE_VALUE;
const VALID: PropertyIdentifier = PropertyIdentifier::VALID_SAMPLES;

fn avg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AVERAGING, 1).unwrap()
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(AveragingObject::new(1, "AVG-1").unwrap()))
            .unwrap();
    })
    .await
}

fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

/// `(property, value bytes)` of a notification about AVG-1, in wire order.
fn values(notification: &COVNotificationRequest) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    assert_eq!(notification.monitored_object_identifier, avg1());
    notification
        .list_of_values
        .iter()
        .map(|value| (value.property_identifier, value.value.clone()))
        .collect()
}

/// SubscribeCOVProperty on one AVG-1 property for `process`.
async fn subscribe_property(
    h: &mut Harness,
    process: u32,
    property: PropertyIdentifier,
    cov_increment: Option<f32>,
) {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: avg1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: property,
        monitored_property_array_index: None,
        cov_increment,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
}

/// The next `count` notifications, keyed by subscriber process.
async fn notifications(
    h: &Harness,
    count: usize,
) -> BTreeMap<u32, Vec<(PropertyIdentifier, Vec<u8>)>> {
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

async fn sample(h: &Harness, value: f32) {
    h.server
        .add_averaging_sample_local(&avg1(), Some(PropertyValue::Real(value)))
        .await
        .unwrap();
}

async fn avg_value(h: &Harness, property: PropertyIdentifier) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&avg1())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
            if c == class && e == code),
        "expected {class:?}/{code:?}, got {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn averaging_sample_local_updates_statistics_and_notifies_property_subscriptions() {
    let mut h = start().await;
    // Before the first sample the statistics hold their empty-window values.
    for (process, property, empty) in [
        (1, MIN, f32::INFINITY),
        (2, MAX, f32::NEG_INFINITY),
        (3, AVG, f32::NAN),
    ] {
        subscribe_property(&mut h, process, property, None).await;
        assert_eq!(response(&h).await, Ok(()), "{property:?} admitted");
        // The initial report has the property alone: no Status_Flags.
        assert_eq!(
            values(&h.cov_notification().await),
            [(property, real(empty))]
        );
    }

    // The first sample moves all three statistics.
    sample(&h, 10.0).await;
    assert_eq!(
        notifications(&h, 3).await,
        BTreeMap::from([
            (1, vec![(MIN, real(10.0))]),
            (2, vec![(MAX, real(10.0))]),
            (3, vec![(AVG, real(10.0))]),
        ])
    );
    // A higher sample leaves Minimum_Value where it was.
    sample(&h, 20.0).await;
    assert_eq!(
        notifications(&h, 2).await,
        BTreeMap::from([(2, vec![(MAX, real(20.0))]), (3, vec![(AVG, real(15.0))])])
    );
    h.no_notification().await;
    // 15 keeps the minimum, the maximum and the average of 10, 20 and 15
    // unchanged, so nobody hears of it; the counts still move.
    sample(&h, 15.0).await;
    h.no_notification().await;
    assert_eq!(
        avg_value(&h, PropertyIdentifier::ATTEMPTED_SAMPLES).await,
        PropertyValue::Unsigned(3)
    );
    assert_eq!(avg_value(&h, VALID).await, PropertyValue::Unsigned(3));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_property_subscription_reports_by_its_cov_increment() {
    let mut h = start().await;
    subscribe_property(&mut h, 7, AVG, Some(5.0)).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(values(&h.cov_notification().await), [(AVG, real(f32::NAN))]);

    sample(&h, 10.0).await; // average 10: moved 10 since the last report
    assert_eq!(values(&h.cov_notification().await), [(AVG, real(10.0))]);
    sample(&h, 12.0).await; // average 11: moved 1
    h.no_notification().await;
    sample(&h, 26.0).await; // average 16: moved 6 since the report of 10
    assert_eq!(values(&h.cov_notification().await), [(AVG, real(16.0))]);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_refuses_subscribe_cov_but_admits_property_multiple() {
    let mut h = start().await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 9,
        monitored_object_identifier: avg1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    assert_eq!(
        response(&h).await,
        Err(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        "Table 13-1 has no Averaging row"
    );
    // Table 12-5 has no Present_Value.
    subscribe_property(&mut h, 10, PV, None).await;
    assert_eq!(response(&h).await, Err(ErrorCode::UNKNOWN_PROPERTY));

    h.subscribe_specs(false, vec![(avg1(), vec![(AVG, false), (VALID, false)])])
        .await;
    assert_eq!(response(&h).await, Ok(()));
    let rows = |notification: COVNotificationMultipleRequest| {
        let [item] = notification.list_of_cov_notifications.as_slice() else {
            panic!("one object in {notification:?}");
        };
        assert_eq!(item.monitored_object_identifier, avg1());
        // A COV-multiple report doesn't order its references; sort them.
        let mut rows = item
            .list_of_values
            .iter()
            .map(|value| (value.property_identifier, value.value.clone()))
            .collect::<Vec<_>>();
        rows.sort_by_key(|(property, _)| property.to_raw());
        rows
    };
    assert_eq!(
        rows(h.notification().await),
        [
            (AVG, real(f32::NAN)),
            (VALID, encoded(PropertyValue::Unsigned(0)))
        ]
    );
    sample(&h, 4.0).await;
    assert_eq!(
        rows(h.notification().await),
        [
            (AVG, real(4.0)),
            (VALID, encoded(PropertyValue::Unsigned(1)))
        ]
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn averaging_sample_local_refuses_bad_values_and_targets() {
    let mut h = start().await;
    sample(&h, 18.0).await;
    subscribe_property(&mut h, 3, AVG, None).await;
    assert_eq!(response(&h).await, Ok(()));
    h.cov_notification().await;

    for (value, code) in [
        (PropertyValue::Double(19.0), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        let error = h
            .server
            .add_averaging_sample_local(&avg1(), Some(value))
            .await
            .unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, code);
    }
    assert_eq!(avg_value(&h, AVG).await, PropertyValue::Real(18.0));
    assert_eq!(
        avg_value(&h, PropertyIdentifier::ATTEMPTED_SAMPLES).await,
        PropertyValue::Unsigned(1),
        "a refused sample isn't counted"
    );

    let unknown = ObjectIdentifier::new(ObjectType::AVERAGING, 9).unwrap();
    let error = h
        .server
        .add_averaging_sample_local(&unknown, Some(PropertyValue::Real(1.0)))
        .await
        .unwrap_err();
    assert_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    let error = h
        .server
        .add_averaging_sample_local(&av1(), Some(PropertyValue::Real(1.0)))
        .await
        .unwrap_err();
    assert_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
