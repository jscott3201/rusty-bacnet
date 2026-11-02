//! Feeding pulses to a registered Accumulator (#2259): `add_pulses_local`
//! steps Present_Value, and the server's monotonic task closes each
//! Limit_Monitoring_Interval window into Pulse_Rate, in paused time.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::accumulator::AccumulatorObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_types::constructed::BACnetPrescale;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;
use tokio::time::Instant;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const RATE: PropertyIdentifier = PropertyIdentifier::PULSE_RATE;

fn acc1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap()
}

/// A server with ACC-1: a 10 s Limit_Monitoring_Interval, Max_Pres_Value
/// 99 and a 1:2 Prescale. Its window opens when the server binds its
/// clock, at the returned instant.
async fn start() -> (Harness, Instant) {
    let origin = Instant::now();
    let h = Harness::start_with(ServerConfig::default(), |db| {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::KILOWATT_HOURS).unwrap();
        acc.set_limit_monitoring_interval(10);
        acc.set_prescale(BACnetPrescale {
            multiplier: 1,
            modulo_divide: 2,
        });
        acc.write_property(
            PropertyIdentifier::MAX_PRES_VALUE,
            None,
            PropertyValue::Unsigned(99),
            None,
        )
        .unwrap();
        db.add(Box::new(acc)).unwrap();
    })
    .await;
    // Startup takes no paused time, so the window opened at `origin`.
    assert_eq!(Instant::now(), origin);
    (h, origin)
}

async fn run_ready() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// Move the paused clock to `millis` after `origin` and let the server run.
async fn advance_to(origin: Instant, millis: u64) {
    let target = origin + Duration::from_millis(millis);
    tokio::time::advance(target.saturating_duration_since(Instant::now())).await;
    run_ready().await;
}

async fn read(h: &Harness, property: PropertyIdentifier) -> PropertyValue {
    let db = h.server.database().read().await;
    db.get(&acc1())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// The deadline ACC-1 reports to the monotonic task, from its origin.
async fn deadline(h: &Harness) -> Option<Duration> {
    let mut db = h.server.database().write().await;
    db.get_mut(&acc1())
        .unwrap()
        .timed()
        .unwrap()
        .next_monotonic_deadline()
}

async fn write(h: &Harness, property: PropertyIdentifier, value: PropertyValue) {
    h.server
        .write_local(
            &acc1(),
            property,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn pulse_rate_follows_the_fed_pulses_each_interval() {
    let (mut h, origin) = start().await;
    assert_eq!(deadline(&h).await, Some(Duration::from_secs(10)));
    h.server.add_pulses_local(&acc1(), 30).await.unwrap();
    advance_to(origin, 4_000).await;
    h.server.add_pulses_local(&acc1(), 15).await.unwrap();
    advance_to(origin, 9_999).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(0));
    advance_to(origin, 10_000).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(45));
    assert_eq!(deadline(&h).await, Some(Duration::from_secs(20)));
    // The next window counts only its own pulses, and a quiet one is zero.
    h.server.add_pulses_local(&acc1(), 7).await.unwrap();
    advance_to(origin, 20_000).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(7));
    advance_to(origin, 30_000).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(0));
    assert_eq!(deadline(&h).await, Some(Duration::from_secs(40)));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn fed_pulses_step_present_value_through_prescale_and_wrap() {
    let (mut h, _origin) = start().await;
    // 1:2 Prescale: 201 pulses are 100 counts with one pulse held, and
    // 100 wraps to 0 past Max_Pres_Value 99.
    h.server.add_pulses_local(&acc1(), 199).await.unwrap();
    assert_eq!(read(&h, PV).await, PropertyValue::Unsigned(99));
    h.server.add_pulses_local(&acc1(), 2).await.unwrap();
    assert_eq!(read(&h, PV).await, PropertyValue::Unsigned(0));
    h.server.add_pulses_local(&acc1(), 1).await.unwrap();
    assert_eq!(read(&h, PV).await, PropertyValue::Unsigned(1));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn out_of_service_keeps_the_clients_values() {
    let (mut h, origin) = start().await;
    // 1:2 Prescale: 21 pulses are 10 counts with one pulse held.
    h.server.add_pulses_local(&acc1(), 21).await.unwrap();
    write(
        &h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    write(&h, RATE, PropertyValue::Unsigned(500)).await;
    // A client's Present_Value starts afresh, without the held pulse (#2267).
    write(&h, PV, PropertyValue::Unsigned(50)).await;
    h.server.add_pulses_local(&acc1(), 40).await.unwrap();
    advance_to(origin, 10_000).await;
    advance_to(origin, 20_000).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(500));
    assert_eq!(read(&h, PV).await, PropertyValue::Unsigned(50));
    // Back in service the next window to close sets the rate again, and
    // Present_Value counts on from the written value: 5 pulses are 2 counts.
    write(
        &h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .await;
    h.server.add_pulses_local(&acc1(), 5).await.unwrap();
    assert_eq!(read(&h, PV).await, PropertyValue::Unsigned(52));
    advance_to(origin, 30_000).await;
    assert_eq!(read(&h, RATE).await, PropertyValue::Unsigned(5));
    h.server.stop().await.unwrap();
}

/// The encoded `property` a notification about ACC-1 carries.
fn value_in(
    notification: &bacnet_services::cov::COVNotificationRequest,
    property: PropertyIdentifier,
) -> Vec<u8> {
    assert_eq!(notification.monitored_object_identifier, acc1());
    notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == property)
        .expect("property reported")
        .value
        .clone()
}

#[tokio::test(start_paused = true)]
async fn a_closed_window_notifies_pulse_rate_subscribers() {
    let (mut h, origin) = start().await;
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: acc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: RATE,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    assert_eq!(response(&h).await, Ok(()));
    // The subscription's initial report carries the rate as it is.
    assert_eq!(
        value_in(&h.cov_notification().await, RATE),
        vec![0x21, 0x00]
    );
    h.server.add_pulses_local(&acc1(), 3).await.unwrap();
    advance_to(origin, 10_000).await;
    assert_eq!(
        value_in(&h.cov_notification().await, RATE),
        vec![0x21, 0x03]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_feed_notifies_present_value_subscribers() {
    let (mut h, _origin) = start().await;
    let mut body = BytesMut::new();
    bacnet_services::cov::SubscribeCOVRequest {
        subscriber_process_identifier: 8,
        monitored_object_identifier: acc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(value_in(&h.cov_notification().await, PV), vec![0x21, 0x00]);
    // 1:2 Prescale: 10 pulses are 5 counts.
    h.server.add_pulses_local(&acc1(), 10).await.unwrap();
    assert_eq!(value_in(&h.cov_notification().await, PV), vec![0x21, 0x05]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_clients_present_value_write_notifies_subscribers() {
    let (mut h, _origin) = start().await;
    let mut body = BytesMut::new();
    bacnet_services::cov::SubscribeCOVRequest {
        subscriber_process_identifier: 9,
        monitored_object_identifier: acc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(value_in(&h.cov_notification().await, PV), vec![0x21, 0x00]);
    // Going out of service reports the Status_Flags change.
    write(
        &h,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert_eq!(value_in(&h.cov_notification().await, PV), vec![0x21, 0x00]);
    // A client's WriteProperty of Present_Value is reported too (#2267).
    let mut body = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: acc1(),
        property_identifier: PV,
        property_array_index: None,
        property_value: vec![0x21, 42],
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(value_in(&h.cov_notification().await, PV), vec![0x21, 42]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn only_an_accumulator_takes_pulses() {
    let (mut h, _origin) = start().await;
    let refused = h.server.add_pulses_local(&av1(), 1).await.unwrap_err();
    assert!(
        matches!(refused, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::OBJECT
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        "{refused:?}"
    );
    let acc9 = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 9).unwrap();
    let unknown = h.server.add_pulses_local(&acc9, 1).await.unwrap_err();
    assert!(
        matches!(unknown, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::OBJECT && code == ErrorCode::UNKNOWN_OBJECT),
        "{unknown:?}"
    );
    h.server.stop().await.unwrap();
}
