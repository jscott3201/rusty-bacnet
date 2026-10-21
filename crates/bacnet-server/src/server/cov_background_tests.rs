//! Background commits reach COV subscribers without another write (#889).
//!
//! Periodic intrinsic reporting, fault detection and schedule writes change
//! objects on their own timers. Each commit fans COV out as a network write
//! does, so ordinary and timestamped subscribers hear of it. Time is paused, and
//! Tokio advances it whenever the test waits.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::Time;

const IN_ALARM: u8 = 0x80;
const FAULT: u8 = 0x40;

/// The Status_Flags bits of an application-tagged BIT STRING value.
fn flags(value: &[u8]) -> u8 {
    assert_eq!(&value[..2], &[0x82, 0x04], "four-bit Status_Flags");
    value[2]
}

fn cov_flags(notification: &COVNotificationRequest) -> u8 {
    let value = notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == SF)
        .expect("Status_Flags reported with Present_Value");
    flags(&value.value)
}

fn cov_value(notification: &COVNotificationRequest) -> Vec<u8> {
    notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == PV)
        .expect("Present_Value reported")
        .value
        .clone()
}

/// Timestamped Status_Flags rows reporting `bit`.
fn flag_rows(notification: &COVNotificationMultipleRequest, bit: u8) -> Vec<Option<Time>> {
    rows(notification)
        .into_iter()
        .filter(|(property, value, _)| *property == SF && flags(value) & bit != 0)
        .map(|(_, _, time)| time)
        .collect()
}

fn database_flags(db: &ObjectDatabase) -> u8 {
    match db.get(&av1()).unwrap().read_property(SF, None).unwrap() {
        PropertyValue::BitString { data, .. } => data[0],
        other => panic!("Status_Flags read as {other:?}"),
    }
}

/// A Schedule commanding AV-1's Present_Value at its fixed priority 16.
fn av1_schedule(db: &mut ObjectDatabase) {
    let mut object = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap();
    object
        .add_object_property_reference(BACnetObjectPropertyReference::new(av1(), PV.to_raw()))
        .unwrap();
    db.add(Box::new(object)).unwrap();
}

/// Change what the schedule commands directly, so only its next evaluation
/// writes AV-1.
async fn command_schedule(h: &Harness, value: f32) {
    let schedule = ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap();
    h.server
        .database()
        .write()
        .await
        .get_mut(&schedule)
        .unwrap()
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Real(value),
            None,
        )
        .unwrap();
}

/// The high-limit alarm, confirmed only after a two-second Time_Delay.
fn delayed_high_limit_alarm(db: &mut ObjectDatabase) {
    high_limit_alarm(db);
    db.get_mut(&av1())
        .unwrap()
        .write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_delayed_alarm_reaches_ordinary_and_timestamped_subscribers_without_another_write() {
    let mut h = Harness::start_with(ServerConfig::default(), delayed_high_limit_alarm).await;
    h.subscribe_cov().await;
    assert_eq!(cov_flags(&h.cov_notification().await), 0);
    h.subscribe(false).await;
    h.notification().await;

    h.set_clock(10);
    h.write_local(90.0).await;
    // The write only starts the Time_Delay countdown.
    assert_eq!(cov_flags(&h.cov_notification().await), 0);
    h.notification().await;

    // The one-second task confirms the alarm at 20, and nothing else is written.
    // Its event notification moves the clock to 30 before COV is prepared.
    h.set_clock(20);
    *h.after_broadcast.lock().unwrap() = Some(at(30));
    let ordinary = h.cov_notification().await;
    assert_eq!(cov_flags(&ordinary) & IN_ALARM, IN_ALARM);
    let timed = h.notification().await;
    assert_eq!(
        *h.clock.0.lock().unwrap(),
        at(30),
        "event notification sent"
    );
    assert_eq!(flag_rows(&timed, IN_ALARM), vec![Some(time(20))]);
    assert_eq!(envelope(&timed), Some((at(20).local_date, time(20))));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn fault_detection_reaches_property_and_timestamped_subscribers_without_another_write() {
    let config = ServerConfig {
        enable_fault_detection: true,
        ..ServerConfig::default()
    };
    let mut h = Harness::start_with(config, |db| {
        db.remove(&av1()).unwrap();
        let mut object =
            AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        object.configure_fault_out_of_range(0.0, 100.0).unwrap();
        // Keep the intrinsic task out: only fault detection may change flags.
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        db.add(Box::new(object)).unwrap();
    })
    .await;
    h.subscribe_cov_property(SF).await;
    assert_eq!(cov_flags(&h.cov_notification().await), 0);
    h.subscribe(false).await;
    h.notification().await;

    h.write_local(150.0).await;
    h.notification().await;
    assert_eq!(database_flags(&*h.server.database().read().await), 0);

    // Fault detection runs every ten seconds.
    h.set_clock(40);
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(cov_flags(&h.cov_notification().await), FAULT);
    assert_eq!(
        flag_rows(&h.notification().await, FAULT),
        vec![Some(time(40))]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_schedule_write_reaches_ordinary_and_timestamped_subscribers() {
    let mut h = Harness::start_with(ServerConfig::default(), av1_schedule).await;
    h.subscribe_cov().await;
    h.cov_notification().await;
    h.subscribe(false).await;
    h.notification().await;

    command_schedule(&h, 42.0).await;
    h.set_clock(33);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(cov_value(&h.cov_notification().await), real(42.0));
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(42.0), Some(time(33)))]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_background_write_that_changes_nothing_reports_nothing() {
    let mut h = Harness::start_with(ServerConfig::default(), av1_schedule).await;
    h.subscribe_cov().await;
    h.cov_notification().await;
    h.subscribe(false).await;
    h.notification().await;
    // Priority 8 holds the value, so the schedule's priority 16 write is masked.
    h.write_local(5.0).await;
    h.cov_notification().await;
    h.notification().await;

    command_schedule(&h, 42.0).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        h.server
            .database()
            .read()
            .await
            .get(&av1())
            .unwrap()
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(16))
            .unwrap(),
        PropertyValue::Real(42.0),
        "the schedule wrote"
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dcc_still_suppresses_background_cov() {
    let mut h = Harness::start_with(ServerConfig::default(), delayed_high_limit_alarm).await;
    h.subscribe_cov().await;
    h.cov_notification().await;
    h.write_local(90.0).await;
    h.cov_notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        database_flags(&*h.server.database().read().await) & IN_ALARM,
        IN_ALARM,
        "detection continued under DCC"
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
