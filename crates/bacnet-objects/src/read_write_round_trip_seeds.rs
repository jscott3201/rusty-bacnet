//! The state `read_write_round_trip_tests` puts each catalogue object in
//! before its round trip (#2272): the writes a row needs before it takes
//! writes at all, and non-empty values, in the wire's whole-list octets,
//! for the lists a new object holds empty.
//!
//! Trend Log Multiple's Log_DeviceObjectProperty stays empty: changing a
//! log's references stamps a log-status record, which needs the device's
//! clock, and a catalogue object has none.

use bacnet_encoding::constructed::{
    encode_access_rule, encode_calendar_entry, encode_destination_list,
    encode_device_object_property_reference, encode_event_notification_subscription_list,
    encode_exception_schedule, encode_setpoint_reference,
};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetCalendarEntry, BACnetDestination, BACnetDeviceObjectPropertyReference,
    BACnetEventNotificationSubscription, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
    Time {
        hour,
        minute,
        second,
        hundredths,
    }
}

/// Writes an object takes before the round trip, so that each row takes
/// writes: event detection on, for Event_Algorithm_Inhibit. An Audit Log
/// gets nothing: its Buffer_Size changes only while logging is off, and
/// turning logging off stamps a log-status record, which needs the clock.
pub(crate) fn prelude(object_type: ObjectType) -> Vec<(P, PropertyValue)> {
    match object_type {
        ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT => {
            vec![(P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true))]
        }
        _ => Vec::new(),
    }
}

/// Non-empty values for the lists `object_type` starts with empty.
pub(crate) fn seeds(object_type: ObjectType) -> Vec<(P, PropertyValue)> {
    let octets = |encode: &dyn Fn(&mut BytesMut)| {
        let mut buf = BytesMut::new();
        encode(&mut buf);
        PropertyValue::ApplicationData(buf.to_vec())
    };
    let oid = |object_type, instance| ObjectIdentifier::new(object_type, instance).unwrap();
    let references = || {
        octets(&|buf| {
            for instance in [1, 2] {
                let reference = BACnetDeviceObjectPropertyReference::new_local(
                    oid(ObjectType::ANALOG_VALUE, instance),
                    P::PRESENT_VALUE.to_raw(),
                );
                encode_device_object_property_reference(buf, &reference);
            }
        })
    };
    let date = |day| Date {
        year: 126,
        month: 11,
        day,
        day_of_week: 0xFF,
    };
    let device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 7));
    let destinations = octets(&|buf| {
        let destination = |process_identifier| BACnetDestination {
            valid_days: DaysOfWeek::all(),
            from_time: time(0, 0, 0, 0),
            to_time: time(23, 59, 59, 99),
            recipient: device.clone(),
            process_identifier,
            issue_confirmed_notifications: false,
            transitions: EventTransitionBits::all(),
        };
        encode_destination_list(buf, &[destination(1), destination(2)]).unwrap();
    });
    match object_type {
        ObjectType::CALENDAR => vec![(
            P::DATE_LIST,
            octets(&|buf| {
                encode_calendar_entry(buf, &BACnetCalendarEntry::Date(date(2)));
                encode_calendar_entry(buf, &BACnetCalendarEntry::Date(date(3)));
            }),
        )],
        ObjectType::SCHEDULE => vec![
            (
                P::EXCEPTION_SCHEDULE,
                octets(&|buf| {
                    let event = BACnetSpecialEvent {
                        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(date(
                            2,
                        ))),
                        list_of_time_values: vec![BACnetTimeValue {
                            time: time(8, 0, 0, 0),
                            value: PropertyValue::Null,
                        }],
                        event_priority: 8,
                    };
                    encode_exception_schedule(buf, &[event]).unwrap();
                }),
            ),
            (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, references()),
        ],
        ObjectType::CHANNEL => vec![(P::LIST_OF_OBJECT_PROPERTY_REFERENCES, references())],
        ObjectType::ACCESS_RIGHTS => {
            let rules = octets(&|buf| {
                encode_access_rule(buf, &BACnetAccessRule::new(None, None, true));
                encode_access_rule(buf, &BACnetAccessRule::new(None, None, false));
            });
            vec![
                (P::POSITIVE_ACCESS_RULES, rules.clone()),
                (P::NEGATIVE_ACCESS_RULES, rules),
            ]
        }
        ObjectType::NOTIFICATION_CLASS => vec![(P::RECIPIENT_LIST, destinations)],
        ObjectType::NOTIFICATION_FORWARDER => vec![
            (P::RECIPIENT_LIST, destinations),
            (
                P::SUBSCRIBED_RECIPIENTS,
                octets(&|buf| {
                    let subscription = |process_identifier| BACnetEventNotificationSubscription {
                        recipient: device.clone(),
                        process_identifier,
                        issue_confirmed_notifications: false,
                        time_remaining: 60,
                    };
                    let both = [subscription(1), subscription(2)];
                    encode_event_notification_subscription_list(buf, &both).unwrap();
                }),
            ),
        ],
        ObjectType::LOOP => vec![(
            P::SETPOINT_REFERENCE,
            octets(&|buf| {
                let reference = BACnetObjectPropertyReference::new(
                    oid(ObjectType::ANALOG_VALUE, 1),
                    P::PRESENT_VALUE.to_raw(),
                );
                encode_setpoint_reference(buf, &reference);
            }),
        )],
        _ => Vec::new(),
    }
}
