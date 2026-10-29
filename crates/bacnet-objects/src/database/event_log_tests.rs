//! `ObjectDatabase::log_event_notification`: each Event Log takes the
//! device's own notifications through its lifecycle (#1275).
use super::*;
use crate::analog::AnalogValueObject;
use crate::clock::{ClockFrame, ClockReader};
use crate::device::{DeviceConfig, DeviceObject};
use crate::event_enrollment::EventEnrollmentObject;
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, NotificationParameters};
use bacnet_types::enums::{
    EngineeringUnits, EventState, EventType, NotifyType, PropertyIdentifier,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

const DATE: Date = Date {
    year: 126,
    month: 9,
    day: 29,
    day_of_week: 2,
};

const TIME: Time = Time {
    hour: 15,
    minute: 4,
    second: 5,
    hundredths: 6,
};

fn frame() -> ClockFrame {
    ClockFrame {
        local_date: DATE,
        local_time: TIME,
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

fn log_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_LOG, instance).unwrap()
}

/// A database on a valid clock holding AV-1 and Event Logs `logs`, each
/// with room for `buffer_size` records.
fn database(logs: &[u32], buffer_size: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedClock(frame()))));
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    for &instance in logs {
        db.add(Box::new(
            EventLogObject::new(instance, format!("EL-{instance}"), buffer_size).unwrap(),
        ))
        .unwrap();
    }
    db
}

/// AV-1's alarm, as the server builds it before any recipient's process
/// identifier is filled in.
fn alarm(event_object: ObjectIdentifier) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 0,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: event_object,
        timestamp: BACnetTimeStamp::SequenceNumber(3),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("high".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    }
}

fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

/// The records `log` holds, through its `records()`.
fn records(db: &mut ObjectDatabase, log: ObjectIdentifier) -> Vec<BACnetEventLogRecord> {
    let object = db
        .objects
        .get_mut(&log)
        .unwrap()
        .as_mut()
        .as_stored_any_mut(crate::traits::ObjectStorageAccess(()))
        .downcast_mut::<EventLogObject>()
        .unwrap();
    object.records().iter().cloned().collect()
}

fn notification_record(notification: EventNotificationRequest) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: DATE,
        time: TIME,
        log_datum: EventLogDatum::Notification(notification),
    }
}

fn status_record(status: LogStatus) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: DATE,
        time: TIME,
        log_datum: EventLogDatum::LogStatus(status),
    }
}

#[test]
fn every_event_log_records_the_notification_at_the_device_clock_time() {
    let mut db = database(&[1, 2], 8);
    db.log_event_notification(&alarm(av1()));
    for log in [log_oid(1), log_oid(2)] {
        assert_eq!(records(&mut db, log), [notification_record(alarm(av1()))]);
    }
}

/// A BUFFER_READY report from `event_object` on the Log_Buffer of
/// `buffer`, in `device` when given.
fn buffer_ready(
    event_object: ObjectIdentifier,
    buffer: ObjectIdentifier,
    device: Option<ObjectIdentifier>,
) -> EventNotificationRequest {
    EventNotificationRequest {
        event_type: EventType::BUFFER_READY,
        notify_type: NotifyType::EVENT,
        to_state: EventState::NORMAL,
        event_values: Some(NotificationParameters::BufferReady {
            buffer_property: BACnetDeviceObjectPropertyReference {
                object_identifier: buffer,
                property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
                property_array_index: None,
                device_identifier: device,
            },
            previous_notification: 0,
            current_notification: 4,
        }),
        ..alarm(event_object)
    }
}

fn trend_log(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, instance).unwrap()
}

/// An Event Log's BUFFER_READY report goes in no log; a Trend Log's goes in
/// every one, and so does any other notification about an Event Log, such
/// as the acknowledgment of a report (#1347).
#[test]
fn only_a_report_on_an_event_log_buffer_goes_in_no_log() {
    let mut db = database(&[1, 2], 8);
    let own = buffer_ready(log_oid(1), log_oid(1), None);
    db.log_event_notification(&own);
    for log in [log_oid(1), log_oid(2)] {
        assert!(records(&mut db, log).is_empty());
    }
    let trend = buffer_ready(trend_log(1), trend_log(1), None);
    let acknowledged = EventNotificationRequest {
        notify_type: NotifyType::ACK_NOTIFICATION,
        event_values: None,
        ..own
    };
    for notification in [trend.clone(), acknowledged.clone()] {
        db.log_event_notification(&notification);
    }
    for log in [log_oid(1), log_oid(2)] {
        assert_eq!(
            records(&mut db, log),
            [
                notification_record(trend.clone()),
                notification_record(acknowledged.clone())
            ]
        );
    }
}

/// Event Enrollment `instance` monitoring `property` of `object`, its
/// reference naming `device` when given.
fn enrollment(
    instance: u32,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    device: Option<u32>,
) -> EventEnrollmentObject {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("EE-{instance}"),
        EventType::CHANGE_OF_VALUE,
    )
    .unwrap();
    let mut reference = BACnetDeviceObjectPropertyReference::new_local(object, property.to_raw());
    reference.device_identifier =
        device.map(|instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap());
    enrollment
        .set_object_property_reference(Some(reference))
        .unwrap();
    enrollment
}

fn ee(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance).unwrap()
}

#[test]
fn an_enrollment_notification_about_any_local_event_log_goes_in_no_log() {
    let mut db = database(&[1, 2], 8);
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::new(7, 0)
                .expect("valid Device identity")
                .with_name("Device-7"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let count = PropertyIdentifier::TOTAL_RECORD_COUNT;
    db.add(Box::new(enrollment(1, log_oid(1), count, None)))
        .unwrap();
    db.add(Box::new(enrollment(2, log_oid(2), count, Some(7))))
        .unwrap();
    // Another device's Event Log 1, and an analog value here.
    db.add(Box::new(enrollment(3, log_oid(1), count, Some(8))))
        .unwrap();
    db.add(Box::new(enrollment(
        4,
        av1(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    )))
    .unwrap();
    for instance in 1..=4 {
        db.log_event_notification(&alarm(ee(instance)));
    }
    // EE-1 and EE-2 watch EL-1 and EL-2: neither log takes either report.
    for log in [log_oid(1), log_oid(2)] {
        assert_eq!(
            records(&mut db, log),
            [3, 4].map(|instance| notification_record(alarm(ee(instance))))
        );
    }
}

#[test]
fn nothing_is_logged_without_a_valid_device_clock() {
    // 2026-09-29 is a Tuesday, not a Wednesday.
    let mut invalid = frame();
    invalid.local_date.day_of_week = 3;
    for clock in [None, Some(invalid)] {
        let mut db = database(&[1], 8);
        db.set_clock_reader(clock.map(|frame| Arc::new(FixedClock(frame)) as Arc<dyn ClockReader>));
        db.log_event_notification(&alarm(av1()));
        assert!(records(&mut db, log_oid(1)).is_empty());
    }
}

#[test]
fn a_disabled_log_ignores_the_notification() {
    let mut db = database(&[1, 2], 8);
    db.get_mut(&log_oid(1))
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    db.log_event_notification(&alarm(av1()));
    assert_eq!(
        records(&mut db, log_oid(1)),
        [status_record(LogStatus::LOG_DISABLED)],
        "only the disable itself is recorded"
    );
    assert_eq!(
        records(&mut db, log_oid(2)),
        [notification_record(alarm(av1()))]
    );
}

#[test]
fn stop_when_full_stops_the_log_before_it_fills() {
    let mut db = database(&[1], 2);
    db.get_mut(&log_oid(1))
        .unwrap()
        .write_property(
            PropertyIdentifier::STOP_WHEN_FULL,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    for _ in 0..3 {
        db.log_event_notification(&alarm(av1()));
    }
    assert_eq!(
        records(&mut db, log_oid(1)),
        [
            notification_record(alarm(av1())),
            status_record(LogStatus::LOG_DISABLED),
        ]
    );
    assert_eq!(
        db.get(&log_oid(1))
            .unwrap()
            .read_property(PropertyIdentifier::LOG_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
}

/// An Event Log type object that refuses every record and counts the tries.
struct Refusing(Arc<AtomicUsize>);

impl BACnetObject for Refusing {
    fn object_identifier(&self) -> ObjectIdentifier {
        log_oid(3)
    }
    fn object_name(&self) -> &str {
        "EL-3"
    }
    fn read_property(&self, _: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        Err(crate::common::unknown_property_error())
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(crate::common::unknown_property_error())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn add_event_log_record(&mut self, _: BACnetEventLogRecord) -> Result<(), Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(Error::protocol(
            ErrorClass::DEVICE,
            ErrorCode::OPERATIONAL_PROBLEM,
        ))
    }
}

#[test]
fn a_log_that_refuses_the_record_leaves_the_others_logging() {
    let mut db = database(&[1, 2], 8);
    let tries = Arc::new(AtomicUsize::new(0));
    db.add(Box::new(Refusing(Arc::clone(&tries)))).unwrap();
    db.log_event_notification(&alarm(av1()));
    assert_eq!(tries.load(Ordering::Relaxed), 1);
    for log in [log_oid(1), log_oid(2)] {
        assert_eq!(records(&mut db, log), [notification_record(alarm(av1()))]);
    }
}

/// Event Log `instance`, taking received notifications when `collects`.
fn received_log(db: &mut ObjectDatabase, instance: u32, collects: bool) {
    let mut log = EventLogObject::new(instance, format!("EL-{instance}"), 8).unwrap();
    log.set_log_received_notifications(collects);
    db.add(Box::new(log)).unwrap();
}

/// Device 50's alarm about its Analog Input 3, as it arrived from there.
fn received() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 9,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap(),
        ..alarm(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap())
    }
}

/// Only a log that opts in takes received notifications, each as it
/// arrived, Process Identifier included (#1346); the opt-in starts off.
#[test]
fn only_a_log_that_opts_in_records_received_notifications() {
    let mut db = database(&[], 8);
    db.add(Box::new(EventLogObject::new(1, "EL-1", 8).unwrap()))
        .unwrap();
    assert!(!db.takes_received_event_notification(&received()));
    db.log_received_event_notification(&received());
    assert!(records(&mut db, log_oid(1)).is_empty());

    received_log(&mut db, 2, true);
    received_log(&mut db, 3, false);
    assert!(db.takes_received_event_notification(&received()));
    db.log_received_event_notification(&received());
    assert!(records(&mut db, log_oid(1)).is_empty());
    assert_eq!(
        records(&mut db, log_oid(2)),
        [notification_record(received())]
    );
    assert!(records(&mut db, log_oid(3)).is_empty());
    // The device's own notifications still go in every log.
    db.log_event_notification(&alarm(av1()));
    assert_eq!(records(&mut db, log_oid(1)).len(), 1);
}

/// A received BUFFER_READY report on an Event Log's buffer, whatever object
/// makes it, and a notification claiming this device as its source, go in
/// no log; any other notification about a remote Event Log goes in.
#[test]
fn received_reports_on_event_log_buffers_and_this_devices_own_go_in_no_log() {
    let mut db = database(&[], 8);
    received_log(&mut db, 1, true);
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::new(7, 0)
                .expect("valid Device identity")
                .with_name("Device-7"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap();
    let device_50 = Some(device);
    let remote = |notification: EventNotificationRequest| EventNotificationRequest {
        initiating_device_identifier: device,
        ..notification
    };
    // Another vendor's Event Enrollment running BUFFER_READY on its Event
    // Log names the log only in Buffer_Property.
    let enrollment_report = remote(buffer_ready(ee(4), log_oid(2), device_50));
    let log_report = remote(buffer_ready(log_oid(2), log_oid(2), device_50));
    let own = EventNotificationRequest {
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
        ..received()
    };
    for notification in [enrollment_report, log_report, own] {
        assert!(!db.takes_received_event_notification(&notification));
        db.log_received_event_notification(&notification);
    }
    assert!(records(&mut db, log_oid(1)).is_empty());
    let unreliable = EventNotificationRequest {
        event_object_identifier: log_oid(2),
        event_type: EventType::CHANGE_OF_RELIABILITY,
        to_state: EventState::FAULT,
        ..received()
    };
    let trend_report = remote(buffer_ready(trend_log(2), trend_log(2), device_50));
    for notification in [unreliable.clone(), trend_report.clone()] {
        assert!(db.takes_received_event_notification(&notification));
        db.log_received_event_notification(&notification);
    }
    assert_eq!(
        records(&mut db, log_oid(1)),
        [
            notification_record(unreliable),
            notification_record(trend_report)
        ]
    );
}
