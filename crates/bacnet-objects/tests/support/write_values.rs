//! Values the clock-bound proptest's targeted pass writes to rows whose
//! current value and its variations never change them: encoded constructed
//! values and the enumerations a row takes (#2262).
//!
//! A row whose datatype is a list of constructed values gets the value two
//! ways, as a list of encoded elements and as one encoding of them all,
//! since rows differ in which they take.

use bacnet_encoding::constructed::{
    encode_access_rule, encode_assigned_landing_calls, encode_daily_schedule, encode_date_range,
    encode_destination, encode_device_object_property_reference,
    encode_event_notification_subscription, encode_event_parameter, encode_fault_parameters,
    encode_landing_door_status, encode_lift_car_call_list, encode_setpoint_reference,
    encode_special_event,
};
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    AssignedLandingCall, BACnetAccessRule, BACnetAssignedLandingCalls, BACnetCalendarEntry,
    BACnetDateRange, BACnetDestination, BACnetDeviceObjectPropertyReference,
    BACnetEventNotificationSubscription, BACnetEventParameter, BACnetLandingDoorStatus,
    BACnetLiftCarCallList, BACnetObjectPropertyReference, BACnetRecipient, BACnetSpecialEvent,
    BACnetTimeValue, FaultParameters, LandingDoor, SpecialEventPeriod,
};
use bacnet_types::enums::{
    AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier, DoorStatus, LiftCarDirection,
    ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

use crate::write_clocks::{Candidate, DATE};

/// The encoding `encode` writes.
pub fn encoded(encode: impl FnOnce(&mut BytesMut)) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode(&mut buf);
    buf.to_vec()
}

/// `elements` as a list of encoded elements, and as one encoding.
pub fn both(elements: Vec<Vec<u8>>) -> Vec<PropertyValue> {
    vec![
        PropertyValue::List(
            elements
                .iter()
                .cloned()
                .map(PropertyValue::ApplicationData)
                .collect(),
        ),
        PropertyValue::ApplicationData(elements.concat()),
    ]
}

/// Analog Value 1's Present_Value.
pub fn av_present_value() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).expect("valid"),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

fn at(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn enumerated(values: &[u32]) -> Vec<PropertyValue> {
    values
        .iter()
        .map(|&value| PropertyValue::Enumerated(value))
        .collect()
}

/// A whole write of each value, through `write_property_from`.
fn whole(values: Vec<PropertyValue>) -> Vec<Candidate> {
    values
        .into_iter()
        .map(|value| (None, value, None, true))
        .collect()
}

/// The crafted values for `property` on `object_type`.
pub fn crafted(object_type: ObjectType, property: P) -> Vec<Candidate> {
    use ObjectType as T;
    let reference = av_present_value();
    let time_values = vec![BACnetTimeValue {
        time: at(9),
        value: PropertyValue::Null,
    }];
    match (object_type, property) {
        (T::SCHEDULE, P::WEEKLY_SCHEDULE) => {
            let day = encoded(|buf| encode_daily_schedule(buf, &time_values).expect("encodable"));
            let empty = encoded(|buf| encode_daily_schedule(buf, &[]).expect("encodable"));
            let mut days = vec![PropertyValue::ApplicationData(empty.clone()); 7];
            days[0] = PropertyValue::ApplicationData(day.clone());
            vec![
                (None, PropertyValue::List(days), None, true),
                (Some(1), PropertyValue::ApplicationData(day), None, true),
            ]
        }
        (T::SCHEDULE, P::EXCEPTION_SCHEDULE) => {
            let event = BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(DATE)),
                list_of_time_values: time_values,
                event_priority: 16,
            };
            whole(both(vec![encoded(|buf| {
                encode_special_event(buf, &event).expect("encodable")
            })]))
        }
        (T::SCHEDULE, P::EFFECTIVE_PERIOD) => {
            let range = BACnetDateRange {
                start_date: DATE,
                end_date: Date {
                    day: 30,
                    day_of_week: 1,
                    ..DATE
                },
            };
            whole(vec![PropertyValue::ApplicationData(encoded(|buf| {
                encode_date_range(buf, &range)
            }))])
        }
        (T::SCHEDULE | T::CHANNEL, P::LIST_OF_OBJECT_PROPERTY_REFERENCES)
        | (T::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY) => {
            whole(both(vec![encoded(|buf| {
                encode_device_object_property_reference(buf, &reference)
            })]))
        }
        (T::CHANNEL, P::EXECUTION_DELAY) => {
            whole(vec![PropertyValue::List(vec![PropertyValue::Unsigned(5)])])
        }
        (T::LOOP, P::SETPOINT_REFERENCE) => {
            let setpoint = BACnetObjectPropertyReference {
                object_identifier: reference.object_identifier,
                property_identifier: reference.property_identifier,
                property_array_index: None,
            };
            whole(vec![PropertyValue::ApplicationData(encoded(|buf| {
                encode_setpoint_reference(buf, &setpoint)
            }))])
        }
        (T::NOTIFICATION_CLASS | T::NOTIFICATION_FORWARDER, P::RECIPIENT_LIST) => {
            let destination = BACnetDestination {
                valid_days: DaysOfWeek::all(),
                from_time: at(0),
                to_time: Time {
                    hour: 23,
                    minute: 59,
                    second: 59,
                    hundredths: 99,
                },
                recipient: BACnetRecipient::Device(
                    ObjectIdentifier::new(ObjectType::DEVICE, 7).expect("valid"),
                ),
                process_identifier: 1,
                issue_confirmed_notifications: false,
                transitions: EventTransitionBits::all(),
            };
            whole(both(vec![encoded(|buf| {
                encode_destination(buf, &destination).expect("encodable")
            })]))
        }
        (T::NOTIFICATION_FORWARDER, P::SUBSCRIBED_RECIPIENTS) => {
            let subscription = BACnetEventNotificationSubscription {
                recipient: BACnetRecipient::Device(
                    ObjectIdentifier::new(ObjectType::DEVICE, 7).expect("valid"),
                ),
                process_identifier: 1,
                issue_confirmed_notifications: false,
                time_remaining: 60,
            };
            whole(both(vec![encoded(|buf| {
                encode_event_notification_subscription(buf, &subscription).expect("encodable")
            })]))
        }
        (T::EVENT_ENROLLMENT, P::EVENT_PARAMETERS) => {
            let parameters = BACnetEventParameter::ChangeOfBitstring {
                time_delay: 5,
                bitmask: (0, vec![0xFF]),
                list_of_values: vec![(0, vec![0x01])],
            };
            whole(vec![PropertyValue::ApplicationData(encoded(|buf| {
                encode_event_parameter(buf, &parameters).expect("encodable")
            }))])
        }
        (T::EVENT_ENROLLMENT, P::FAULT_PARAMETERS) => {
            let parameters = FaultParameters::FaultCharacterString {
                fault_values: vec!["fault".into()],
            };
            whole(vec![PropertyValue::ApplicationData(encoded(|buf| {
                encode_fault_parameters(buf, &parameters).expect("encodable")
            }))])
        }
        (T::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES | P::NEGATIVE_ACCESS_RULES) => {
            let rule = BACnetAccessRule {
                time_range_specifier: AccessRuleTimeRangeSpecifier::ALWAYS,
                time_range: None,
                location_specifier: AccessRuleLocationSpecifier::ALL,
                location: None,
                enable: true,
            };
            whole(both(vec![encoded(|buf| encode_access_rule(buf, &rule))]))
        }
        (T::TREND_LOG, P::CLIENT_COV_INCREMENT) => whole(vec![PropertyValue::Real(0.5)]),
        (T::LIGHTING_OUTPUT, P::DEFAULT_RAMP_RATE) => whole(vec![PropertyValue::Real(50.0)]),
        // Door alarm states: ALARM, FORCED_OPEN, DOOR_FAULT and the rest.
        (T::ACCESS_DOOR, P::DOOR_ALARM_STATE) => whole(enumerated(&[1, 2, 3, 4, 5, 6, 7, 8])),
        (T::ACCESS_DOOR, P::ALARM_VALUES | P::MASKED_ALARM_VALUES) => {
            whole(vec![PropertyValue::List(enumerated(&[1, 3]))])
        }
        (T::ACCESS_DOOR, P::FAULT_VALUES) => whole(vec![PropertyValue::List(enumerated(&[5]))]),
        // Zone occupancy states above the upper limit.
        (T::ACCESS_ZONE, P::ALARM_VALUES) => whole(vec![PropertyValue::List(enumerated(&[4]))]),
        (T::ESCALATOR | T::LIFT, P::FAULT_SIGNALS) => {
            whole(vec![PropertyValue::List(enumerated(&[1]))])
        }
        (T::LIFT, P::ASSIGNED_LANDING_CALLS) => {
            let calls = BACnetAssignedLandingCalls {
                landing_calls: vec![AssignedLandingCall {
                    floor_number: 1,
                    direction: LiftCarDirection::UP,
                }],
            };
            let element = encoded(|buf| encode_assigned_landing_calls(buf, &calls));
            vec![(Some(1), PropertyValue::ApplicationData(element), None, true)]
        }
        (T::LIFT, P::REGISTERED_CAR_CALL) => {
            let calls = BACnetLiftCarCallList {
                floor_numbers: vec![2],
            };
            let element = encoded(|buf| encode_lift_car_call_list(buf, &calls));
            vec![(Some(1), PropertyValue::ApplicationData(element), None, true)]
        }
        (T::LIFT, P::LANDING_DOOR_STATUS) => {
            let status = BACnetLandingDoorStatus {
                landing_doors: vec![LandingDoor {
                    floor_number: 1,
                    door_status: DoorStatus::OPENED,
                }],
            };
            let element = encoded(|buf| encode_landing_door_status(buf, &status));
            vec![(Some(1), PropertyValue::ApplicationData(element), None, true)]
        }
        _ => Vec::new(),
    }
}
