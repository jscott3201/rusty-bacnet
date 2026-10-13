use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_encoding::constructed::encode_event_log_record;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetPropertyValue, EventLogDatum, EventNotificationRequest, NotificationParameters,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, EventType, NotifyType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, StatusFlags, Time};
use bytes::BytesMut;
use std::sync::Arc;

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: make_date(),
            local_time: make_time(9),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn bind_clock(object: &mut EventLogObject) {
    object.bind_clock_internal(Some(Arc::new(FixedClock)));
}

fn make_date() -> Date {
    Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    }
}

fn make_time(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

/// A clock-change record, the one ordinary kind that needs no notification.
fn make_record(hour: u8, value: f32) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: make_date(),
        time: make_time(hour),
        log_datum: EventLogDatum::TimeChange(value),
    }
}

/// A short change-of-state alarm from Device 1 about Analog Input 1, with
/// `event_values` as given.
fn notification(
    event_type: EventType,
    event_values: Option<NotificationParameters>,
) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values,
    }
}

fn framed(record: &BACnetEventLogRecord) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_log_record(record, &mut buf).unwrap();
    buf.to_vec()
}

/// Each resident record as ReadRange serves it.
fn served(el: &EventLogObject) -> Vec<Vec<u8>> {
    let records = el.log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut buf = BytesMut::new();
            records.encode_record(index, &mut buf);
            buf.to_vec()
        })
        .collect()
}

fn assert_read_access_denied(result: Result<PropertyValue, Error>) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::READ_ACCESS_DENIED.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / READ_ACCESS_DENIED, got {other:?}"),
    }
}

#[test]
fn create_event_log() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    assert_eq!(el.object_identifier().object_type(), ObjectType::EVENT_LOG);
    assert_eq!(el.object_identifier().instance_number(), 1);
    assert_eq!(el.object_name(), "EL-1");
}

#[test]
fn read_object_type() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let val = el
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::EVENT_LOG.to_raw())
    );
}

#[test]
fn add_records_and_read_count() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.add_record(make_record(10, 72.5)).unwrap();
    el.add_record(make_record(11, 73.0)).unwrap();
    assert_eq!(el.records().len(), 2);
    let val = el
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
    let val = el
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
}

/// Clause 12.27.13 opens the buffer to ReadRange only (#1237): ReadProperty
/// refuses it, empty or not, while the records stay available to ReadRange.
#[test]
fn read_property_refuses_log_buffer() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    assert_read_access_denied(el.read_property(PropertyIdentifier::LOG_BUFFER, None));
    el.add_record(make_record(10, 72.5)).unwrap();
    el.add_record(make_record(11, 73.0)).unwrap();
    assert_read_access_denied(el.read_property(PropertyIdentifier::LOG_BUFFER, None));
    assert_eq!(el.log_buffer_internal().unwrap().record_count(), 2);
}

/// Each record is served framed as Clause 21's BACnetEventLogRecord (#1233).
#[test]
fn log_buffer_records_are_framed_event_log_records() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.add_record(make_record(10, 72.5)).unwrap();
    assert_eq!(
        served(&el),
        vec![vec![
            0x0E, 0xA4, 0x7C, 0x03, 0x0F, 0x05, 0xB4, 0x0A, 0x00, 0x00, 0x00,
            0x0F, // timestamp
            0x1E, 0x2C, 0x42, 0x91, 0x00, 0x00, 0x1F, // time-change [2]: 72.5 s
        ]]
    );
}

/// A record that would not encode is refused when it is added, so the log
/// never holds one that a ReadRange window could not serve.
#[test]
fn unencodable_record_is_refused_at_add_and_the_rest_still_serve() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.add_record(make_record(10, 72.5)).unwrap();
    for (event_type, event_values) in [
        // Raw event values with an opening tag left open.
        (
            EventType::COMMAND_FAILURE,
            NotificationParameters::CommandFailure {
                command_value: vec![0x3E, 0x19, 0x05],
                status_flags: StatusFlags::empty(),
                feedback_value: vec![0x91, 0x00],
            },
        ),
        // A property priority outside 1 to 16.
        (
            EventType::EXTENDED,
            NotificationParameters::ComplexEventType {
                property_values: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x10],
                    priority: Some(17),
                }],
            },
        ),
        // A bit string with more than seven unused bits, which the record
        // decoder would refuse.
        (
            EventType::CHANGE_OF_BITSTRING,
            NotificationParameters::ChangeOfBitstring {
                referenced_bitstring: (8, vec![0xA0]),
                status_flags: StatusFlags::empty(),
            },
        ),
    ] {
        let bad = BACnetEventLogRecord {
            date: make_date(),
            time: make_time(11),
            log_datum: EventLogDatum::Notification(notification(event_type, Some(event_values))),
        };
        assert!(el.add_record(bad).is_err());
    }
    el.add_record(make_record(12, 73.0)).unwrap();
    assert_eq!(el.records().len(), 2);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        served(&el),
        vec![
            framed(&make_record(10, 72.5)),
            framed(&make_record(12, 73.0))
        ]
    );
}

/// A restore (#1537) refuses a record that would not encode as `add_record`
/// does, and keeps the log as it was.
#[test]
fn restore_refuses_an_unencodable_record() {
    let mut el = EventLogObject::new(1, "EL-1", 5).unwrap();
    el.add_record(make_record(10, 72.5)).unwrap();
    let records = el.records().clone();
    let open = BACnetEventLogRecord {
        date: make_date(),
        time: make_time(11),
        log_datum: EventLogDatum::Notification(notification(
            EventType::COMMAND_FAILURE,
            Some(NotificationParameters::CommandFailure {
                command_value: vec![0x3E, 0x19, 0x05],
                status_flags: StatusFlags::empty(),
                feedback_value: vec![0x91, 0x00],
            }),
        )),
    };
    el.restore_log_buffer(7, [make_record(12, 73.0), open])
        .unwrap_err();
    assert_eq!(el.records(), &records);
    assert_eq!(el.total_record_count(), 1);
}

#[test]
fn ring_buffer_wraps() {
    let mut el = EventLogObject::new(1, "EL-1", 3).unwrap();
    for i in 0..5u8 {
        el.add_record(make_record(i, f32::from(i))).unwrap();
    }
    assert_eq!(el.records().len(), 3);
    // Oldest records evicted; first remaining is hour=2
    assert_eq!(el.records()[0].time.hour, 2);
    let val = el
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
}

#[test]
fn stop_when_full() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        el.add_record(make_record(i, i as f32)).unwrap();
    }
    assert_eq!(el.records().len(), 2);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    ); // Only 2 accepted
}

#[test]
fn disable_logging() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    el.add_record(make_record(10, 72.5)).unwrap();
    assert_eq!(el.records().len(), 1);
    assert_eq!(
        el.records()[0].log_datum,
        EventLogDatum::LogStatus(LogStatus::LOG_DISABLED)
    );
}

#[test]
fn clear_buffer_via_record_count() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    bind_clock(&mut el);
    el.add_record(make_record(10, 72.5)).unwrap();
    assert_eq!(el.records().len(), 1);
    el.write_property(
        PropertyIdentifier::RECORD_COUNT,
        None,
        PropertyValue::Unsigned(0),
        None,
    )
    .unwrap();
    assert_eq!(el.records().len(), 1);
    assert_eq!(
        el.records()[0].log_datum,
        EventLogDatum::LogStatus(LogStatus::BUFFER_PURGED)
    );
}

#[test]
fn read_event_state_default() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let val = el
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn property_list_complete() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let props = el.property_list();
    assert!(props.contains(&PropertyIdentifier::LOG_ENABLE));
    assert!(props.contains(&PropertyIdentifier::STOP_WHEN_FULL));
    assert!(props.contains(&PropertyIdentifier::BUFFER_SIZE));
    assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
    assert!(props.contains(&PropertyIdentifier::RECORD_COUNT));
    assert!(props.contains(&PropertyIdentifier::TOTAL_RECORD_COUNT));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    // Table 12-31 has neither of these rows (#1064).
    assert!(!props.contains(&PropertyIdentifier::LOG_INTERVAL));
    assert!(!props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
}

/// Table 12-31 defines neither Out_Of_Service nor Log_Interval (#1064, as
/// #985 did for the Trend Logs): reads and writes find no property, and the
/// OUT_OF_SERVICE status flag stays clear.
#[test]
fn event_log_has_no_out_of_service_or_log_interval() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    for (property, value) in [
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::LOG_INTERVAL,
            PropertyValue::Unsigned(60),
        ),
    ] {
        assert!(!el.is_writable_property(property));
        let read = el.read_property(property, None).map(|_| ());
        let write = el.write_property(property, None, value, None);
        for result in [read, write] {
            match result {
                Err(Error::Protocol { class, code }) => {
                    assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                    assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32);
                }
                other => panic!("{property:?}: expected UNKNOWN_PROPERTY, got {other:?}"),
            }
        }
    }
    assert_eq!(
        el.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );
}

#[test]
fn write_absent_property_is_unknown() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let result = el.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(
        matches!(result, Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32 && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
    );
}

#[test]
fn log_buffer_serves_every_event_log_datum() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let records = [
        EventLogDatum::Notification(notification(EventType::CHANGE_OF_STATE, None)),
        EventLogDatum::TimeChange(-0.5),
        EventLogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
    ]
    .map(|log_datum| BACnetEventLogRecord {
        date: make_date(),
        time: make_time(8),
        log_datum,
    });
    for record in &records {
        el.add_record(record.clone()).unwrap();
    }
    assert_eq!(el.records(), &records);
    assert_eq!(served(&el), records.iter().map(framed).collect::<Vec<_>>());
}

#[test]
fn event_log_identities_align_after_eviction_and_differ_from_position() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    for hour in 1..=3 {
        el.add_record(make_record(hour, hour as f32)).unwrap();
    }

    let identities = el.log_record_identities_internal().unwrap();
    let wire = served(&el);
    assert_eq!(identities.len(), el.records().len());
    assert_eq!(identities.len(), wire.len());
    assert_eq!(identities[0].sequence_number(), 2);
    assert_ne!(identities[0].sequence_number(), 1);
    for ((identity, raw), wire) in identities.iter().zip(el.records()).zip(wire) {
        assert_eq!(identity.date(), raw.date);
        assert_eq!(identity.time(), raw.time);
        assert_eq!(
            wire,
            framed(&make_record(raw.time.hour, raw.time.hour as f32))
        );
    }
}

#[test]
fn event_log_clear_preserves_total_and_next_identity() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    el.add_record(make_record(1, 1.0)).unwrap();
    el.add_record(make_record(1, 2.0)).unwrap();
    assert_eq!(
        el.log_record_identities_internal()
            .unwrap()
            .iter()
            .map(|identity| identity.sequence_number())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    el.clear();
    assert!(el.log_record_identities_internal().unwrap().is_empty());
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    el.add_record(make_record(1, 3.0)).unwrap();
    assert_eq!(
        el.log_record_identities_internal().unwrap()[0].sequence_number(),
        3
    );
}

#[test]
fn event_log_disabled_ordinary_rejection_does_not_consume_identity() {
    let mut el = EventLogObject::new(1, "EL-1", 1).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let before = el.log_record_identities_internal().unwrap();
    el.add_record(make_record(1, 1.0)).unwrap();

    assert_eq!(el.log_record_identities_internal().unwrap(), before);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn event_log_total_record_count_is_u32_and_wraps_max_to_one() {
    let mut el = EventLogObject::new(1, "EL-1", 1).unwrap();
    el.restore_log_buffer(u32::MAX, []).unwrap();
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(u32::MAX as u64)
    );
    el.add_record(make_record(1, 1.0)).unwrap();
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        el.log_record_identities_internal().unwrap()[0].sequence_number(),
        1
    );
}
