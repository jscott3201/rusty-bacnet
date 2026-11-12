//! A Time_Delay runs from the instant its condition began (#2303): a write
//! between the one-second sweeps seeds a transition due its delay after the
//! write, and the sweep task wakes for it then, not at a whole second.

use super::event_notifications_tests::{
    decode_broadcast_notification, notification_class_0_broadcasting, recording_transport,
};
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState};

/// Analog Inputs AI-1, AI-2, ... reading 50, inside their limits, each
/// reporting every transition of OUT_OF_RANGE through a broadcasting
/// Notification Class 0 after its own Time_Delay.
fn database(delays: &[u32]) -> (ObjectDatabase, Vec<ObjectIdentifier>) {
    let mut db = clocked_test_database();
    let mut oids = Vec::new();
    for (instance, time_delay) in (1..).zip(delays) {
        let name = format!("AI-{instance}");
        let mut ai =
            AnalogInputObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
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
            PropertyValue::Unsigned((*time_delay).into()),
            None,
        )
        .unwrap();
        ai.set_present_value(50.0);
        oids.push(ai.object_identifier());
        db.add(Box::new(ai)).unwrap();
    }
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
    (db, oids)
}

/// Lower `oid`'s High_Limit below its value through the server's write
/// path, so its condition begins now and its delay is seeded.
async fn begin_condition(server: &BACnetServer<TestTransport>, oid: &ObjectIdentifier) {
    server
        .write_local(
            oid,
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("the local write seeds the delayed transition");
}

/// The instances whose EventNotifications have gone out, sorted: one sweep
/// sends in the database's order.
fn notified(sent: &SendLog) -> Vec<u32> {
    use bacnet_encoding::apdu::decode_apdu;
    use bacnet_encoding::constructed::decode_event_notification;
    use bacnet_encoding::npdu::decode_npdu;

    let mut instances: Vec<u32> = sent
        .npdus()
        .into_iter()
        .filter_map(|frame| {
            let npdu = decode_npdu(frame).ok()?;
            let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).ok()? else {
                return None;
            };
            (request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION)
                .then(|| decode_event_notification(&request.service_request).unwrap())
        })
        .map(|notification| notification.event_object_identifier.instance_number())
        .collect();
    instances.sort_unstable();
    instances
}

/// Written 0.9 s after the sweeps began, with Time_Delay 2, the alarm goes
/// out at 2.9 s. The one-second countdown sent it at the 2 s sweep, 1.1 s
/// after the write.
#[tokio::test(start_paused = true)]
async fn a_write_seeded_delay_ends_two_seconds_after_the_write() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[2]);
    let oid = oids[0];
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

/// Two writes' delays end in turn, each at its own instant, and a
/// Time_Delay of 0 the 1 s sweep noticed, pending with its due instant
/// already reached, delays neither of them: it goes out at the next sweep,
/// the one woken for the first write's delay.
#[tokio::test(start_paused = true)]
async fn each_delay_ends_at_its_own_instant() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1, 0]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    // AI-3's condition begins with no write the server sees.
    tokio::time::sleep_until(at(100)).await;
    server
        .database()
        .write()
        .await
        .get_mut(&oids[2])
        .unwrap()
        .write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
        )
        .unwrap();
    tokio::time::sleep_until(at(200)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.2 s
    tokio::time::sleep_until(at(500)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.5 s

    for (millis, expected) in [
        (1_190, &[][..]),
        (1_210, &[1, 3]),
        (1_490, &[1, 3]),
        (1_510, &[1, 2, 3]),
    ] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// A sweep woken for a due instant runs no sooner than the floor after the
/// last one: delays ending 50 ms apart go out 100 ms apart.
#[tokio::test(start_paused = true)]
async fn a_due_wake_waits_out_the_floor() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    tokio::time::sleep_until(at(200)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.2 s
    tokio::time::sleep_until(at(250)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.25 s

    for (millis, expected) in [(1_210, &[1][..]), (1_290, &[1]), (1_310, &[1, 2])] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// An object that counts the sweeps and always reports the same past due
/// instant, as a transition whose commit keeps failing would.
struct SweepCounter {
    oid: ObjectIdentifier,
    sweeps: Arc<std::sync::atomic::AtomicUsize>,
    /// A due instant before the first sweep, close to the clock so it maps
    /// to the host's timeline on every platform.
    past: bacnet_objects::event::Instant,
}

impl BACnetObject for SweepCounter {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "SWEEP-COUNTER"
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(
        &self,
    ) -> std::borrow::Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
    fn event_reporting(&self) -> Option<&dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
}

impl bacnet_objects::traits::EventReporting for SweepCounter {
    fn tick_intrinsic_reporting(
        &mut self,
        _now: bacnet_objects::event::Instant,
    ) -> Option<bacnet_objects::event::TransitionOutcome> {
        self.sweeps.fetch_add(1, Ordering::SeqCst);
        None
    }
    fn next_due(&self) -> Option<bacnet_objects::event::Instant> {
        Some(self.past)
    }
}

/// A due instant already past adds no sweep: the sweep that saw it has
/// run, so the task keeps its one-second cadence rather than waking again
/// at every floor.
#[tokio::test(start_paused = true)]
async fn a_past_due_instant_adds_no_sweep() {
    let (transport, _sent) = recording_transport();
    let (mut db, _) = database(&[]);
    let sweeps = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    db.add(Box::new(SweepCounter {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap(),
        sweeps: Arc::clone(&sweeps),
        past: crate::runtime_clock::core_now()
            .saturating_sub(bacnet_objects::event::Duration::from_secs(2)),
    }))
    .unwrap();
    let _server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();

    tokio::time::sleep_until(start + Duration::from_millis(3_500)).await;
    // The sweeps at 0, 1, 2 and 3 s.
    assert_eq!(sweeps.load(Ordering::SeqCst), 4);
}

/// The floor never delays the periodic sweep: after a sweep woken at 1.95 s
/// for one delay, a second delay ending at 1.98 s goes out with the
/// periodic sweep at 2 s, not at 2.05 s.
#[tokio::test(start_paused = true)]
async fn the_floor_never_delays_the_periodic_sweep() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    tokio::time::sleep_until(at(950)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.95 s
    tokio::time::sleep_until(at(980)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.98 s

    for (millis, expected) in [
        (1_940, &[][..]),
        (1_960, &[1]),
        (1_990, &[1]),
        (2_010, &[1, 2]),
    ] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}
