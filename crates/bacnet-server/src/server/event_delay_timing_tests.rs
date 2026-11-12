//! A Time_Delay runs from the instant its condition began (#2303): a write
//! between the one-second sweeps seeds a transition due its delay after the
//! write, and the sweep task wakes for it then, not at a whole second.

use super::event_notifications_tests::{
    decode_broadcast_notification, notification_class_0_broadcasting, recording_transport,
};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState};

/// An Analog Input reading 50, inside its limits, reporting every transition
/// of OUT_OF_RANGE through a broadcasting Notification Class 0 after
/// `time_delay` seconds.
fn delayed_database(time_delay: u32) -> (ObjectDatabase, ObjectIdentifier) {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [
        (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
        (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
    ] {
        ai.write_property(
            property,
            None,
            PropertyValue::BitString {
                unused_bits,
                data: vec![bits],
            },
            None,
        )
        .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(time_delay.into()),
        None,
    )
    .unwrap();
    ai.set_present_value(50.0);
    let oid = ai.object_identifier();

    let mut db = clocked_test_database();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    (db, oid)
}

/// Written 0.9 s after the sweeps began, with Time_Delay 2, the alarm goes
/// out at 2.9 s. The one-second countdown sent it at the 2 s sweep, 1.1 s
/// after the write.
#[tokio::test(start_paused = true)]
async fn a_write_seeded_delay_ends_two_seconds_after_the_write() {
    let (transport, sent) = recording_transport();
    let (db, oid) = delayed_database(2);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();

    tokio::time::sleep_until(start + Duration::from_millis(900)).await;
    // Lowering High_Limit below the value begins the condition now.
    server
        .write_local(
            &oid,
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("the local write seeds the delayed transition");

    tokio::time::sleep_until(start + Duration::from_millis(2_890)).await;
    assert!(
        sent.npdus().is_empty(),
        "the alarm went out before its delay ended"
    );

    tokio::time::sleep_until(start + Duration::from_millis(2_910)).await;
    let notification = decode_broadcast_notification(&sent.npdus());
    assert_eq!(notification.from_state, EventState::NORMAL);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
}
