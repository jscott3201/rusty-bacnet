//! Every event type's notification values, as text and as typed data
//! (#2073).

use bacnet_encoding::constructed::{encode_authentication_factor, encode_bacnet_property_value};
use bacnet_types::constructed::{
    BACnetAuthenticationFactor, BACnetDeviceObjectPropertyReference, BACnetPropertyStates,
};
use bacnet_types::enums::{
    AccessEvent, AuthenticationFactorType, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    PropertyIdentifier, Reliability, TimerState, TimerTransition,
};
use bacnet_types::primitives::{ObjectIdentifier, StatusFlags};
use bytes::BytesMut;
use serde_json::json;

use super::*;

const ALARM: StatusFlags = StatusFlags::IN_ALARM;
const NONE: StatusFlags = StatusFlags::empty();

fn real(value: f32) -> Vec<u8> {
    [vec![0x44], value.to_be_bytes().to_vec()].concat()
}

fn stamp() -> (Date, Time) {
    (
        Date {
            year: 126,
            month: 10,
            day: 25,
            day_of_week: 7,
        },
        Time {
            hour: 14,
            minute: 30,
            second: 5,
            hundredths: 0,
        },
    )
}

fn present_value(priority: Option<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: real(72.5),
        priority,
    }
}

/// Every event type the stack decodes, with the text its values show.
fn cases() -> Vec<(NotificationParameters, &'static str)> {
    use NotificationParameters as N;
    let mut factor = BytesMut::new();
    encode_authentication_factor(
        &mut factor,
        &BACnetAuthenticationFactor {
            format_type: AuthenticationFactorType::SIMPLE_NUMBER16,
            format_class: 0,
            value: vec![0x04, 0xD2],
        },
    );
    let mut reliability_values = BytesMut::new();
    encode_bacnet_property_value(&present_value(None), &mut reliability_values);
    vec![
        (
            N::ChangeOfBitstring {
                referenced_bitstring: (4, vec![0xA0]),
                status_flags: ALARM,
            },
            "change-of-bitstring: bits bits(1010), flags in-alarm",
        ),
        (
            N::ChangeOfState {
                new_state: BACnetPropertyStates::State(3),
                status_flags: NONE,
            },
            "change-of-state: state high-limit, flags none",
        ),
        (
            N::ChangeOfValue {
                new_value: ChangeOfValueChoice::ChangedValue(72.5),
                status_flags: NONE,
            },
            "change-of-value: value 72.5, flags none",
        ),
        (
            N::CommandFailure {
                command_value: real(50.0),
                status_flags: StatusFlags::OVERRIDDEN,
                feedback_value: real(30.0),
            },
            "command-failure: commanded 50, flags overridden, feedback 30",
        ),
        (
            N::FloatingLimit {
                reference_value: 80.5,
                status_flags: ALARM,
                setpoint_value: 70.0,
                error_limit: 5.0,
            },
            "floating-limit: value 80.5, flags in-alarm, setpoint 70, error limit 5",
        ),
        (
            N::OutOfRange {
                exceeding_value: 101.5,
                status_flags: ALARM,
                deadband: 1.0,
                exceeded_limit: 100.0,
            },
            "out-of-range: exceeding value 101.5, flags in-alarm, deadband 1, limit 100",
        ),
        (
            N::ComplexEventType {
                property_values: vec![present_value(Some(8))],
            },
            "complex-event-type: values [present-value = 72.5 @8]",
        ),
        (
            N::ChangeOfLifeSafety {
                new_state: LifeSafetyState::ALARM,
                new_mode: LifeSafetyMode::ON,
                status_flags: ALARM,
                operation_expected: LifeSafetyOperation::SILENCE,
            },
            "change-of-life-safety: state alarm, mode on, flags in-alarm, operation silence",
        ),
        (
            N::Extended {
                vendor_id: 555,
                extended_event_type: 9,
                parameters: vec![0x21, 0x05],
            },
            "extended: vendor 555 type 9, parameters 5",
        ),
        (
            N::BufferReady {
                buffer_property: BACnetDeviceObjectPropertyReference::new_local(
                    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
                    PropertyIdentifier::LOG_BUFFER.to_raw(),
                ),
                previous_notification: 10,
                current_notification: 20,
            },
            "buffer-ready: buffer trend-log 1 log-buffer, previous 10, current 20",
        ),
        (
            N::UnsignedRange {
                exceeding_value: 301,
                status_flags: ALARM,
                exceeded_limit: 300,
            },
            "unsigned-range: exceeding value 301, flags in-alarm, limit 300",
        ),
        (
            N::AccessEvent {
                access_event: AccessEvent::GRANTED,
                status_flags: NONE,
                access_event_tag: 7,
                access_event_time: stamp(),
                access_credential: BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1)
                        .unwrap(),
                },
                authentication_factor: Some(factor.to_vec()),
            },
            "access-event: event granted, flags none, tag 7, time 2026-10-25 Sun 14:30:05, \
             credential access-credential 1, factor simple-number16 class 0 value 0x04d2",
        ),
        (
            N::DoubleOutOfRange {
                exceeding_value: -2.5,
                status_flags: ALARM,
                deadband: 0.5,
                exceeded_limit: -2.0,
            },
            "double-out-of-range: exceeding value -2.5, flags in-alarm, deadband 0.5, limit -2",
        ),
        (
            N::SignedOutOfRange {
                exceeding_value: -6,
                status_flags: ALARM,
                deadband: 1,
                exceeded_limit: -5,
            },
            "signed-out-of-range: exceeding value -6, flags in-alarm, deadband 1, limit -5",
        ),
        (
            N::UnsignedOutOfRange {
                exceeding_value: 301,
                status_flags: ALARM,
                deadband: 1,
                exceeded_limit: 300,
            },
            "unsigned-out-of-range: exceeding value 301, flags in-alarm, deadband 1, limit 300",
        ),
        (
            N::ChangeOfCharacterstring {
                changed_value: "FIRE".into(),
                status_flags: ALARM,
                alarm_value: "FIRE".into(),
            },
            "change-of-characterstring: value \"FIRE\", flags in-alarm, alarm value \"FIRE\"",
        ),
        (
            N::ChangeOfStatusFlags {
                present_value: Some(vec![0x21, 0x02]),
                referenced_flags: StatusFlags::FAULT,
            },
            "change-of-status-flags: value 2, flags fault",
        ),
        (
            N::ChangeOfReliability {
                reliability: Reliability::OVER_RANGE,
                status_flags: StatusFlags::FAULT,
                property_values: reliability_values.to_vec(),
            },
            "change-of-reliability: reliability over-range, flags fault, \
             values [present-value = 72.5]",
        ),
        (
            N::ChangeOfDiscreteValue {
                new_value: vec![0x21, 0x03],
                status_flags: NONE,
            },
            "change-of-discrete-value: value 3, flags none",
        ),
        (
            N::ChangeOfTimer {
                new_state: TimerState::EXPIRED,
                status_flags: ALARM,
                update_time: stamp(),
                last_state_change: Some(TimerTransition::RUNNING_TO_EXPIRED),
                initial_timeout: Some(60),
                expiration_time: None,
            },
            "change-of-timer: state expired, flags in-alarm, updated 2026-10-25 Sun 14:30:05, \
             last change running-to-expired, timeout 60 s",
        ),
    ]
}

fn json_of(values: &EventValues) -> serde_json::Value {
    serde_json::to_value(&values.data).unwrap()
}

/// Each event type: the text, the event type first in the fields, and the
/// text and details agreeing.
#[test]
fn every_event_type_shows_its_values() {
    let cases = cases();
    assert_eq!(cases.len(), 20, "every NotificationParameters choice");
    for (parameters, text) in cases {
        let values = EventValues::new(&parameters);
        assert_eq!(values.text(), text);
        assert_eq!(format!("{}: {}", values.event_type, values.details), text);
        let json = json_of(&values);
        assert_eq!(json["element"], "notification-parameters");
        assert_eq!(json["text"], text);
        assert_eq!(
            json["fields"]["event_type"]["value"],
            event_type(&parameters),
            "{text}"
        );
        assert_eq!(json["fields"]["event_type"]["name"], values.event_type);
    }
}

/// The typed data of each event type, field by field.
#[test]
fn every_event_type_has_its_fields() {
    let alarm = json!({"bits": "1000", "names": ["in-alarm"]});
    let none = json!({"bits": "0000", "names": []});
    let fault = json!({"bits": "0100", "names": ["fault"]});
    let present = json!({
        "element": "property-value",
        "text": "present-value = 72.5 @8",
        "fields": {
            "property_identifier": {"value": 85, "name": "present-value"},
            "property_array_index": null,
            "value": 72.5,
            "priority": 8,
        },
    });
    let expected = [
        json!({"event_type": {"value": 0, "name": "change-of-bitstring"},
               "referenced_bitstring": {"bits": "1010"}, "status_flags": alarm}),
        json!({"event_type": {"value": 1, "name": "change-of-state"},
               "new_state": {"element": "property-state", "text": "high-limit"},
               "status_flags": none}),
        json!({"event_type": {"value": 2, "name": "change-of-value"},
               "changed_value": 72.5, "status_flags": none}),
        json!({"event_type": {"value": 3, "name": "command-failure"},
               "command_value": 50.0,
               "status_flags": {"bits": "0010", "names": ["overridden"]},
               "feedback_value": 30.0}),
        json!({"event_type": {"value": 4, "name": "floating-limit"},
               "reference_value": 80.5, "status_flags": alarm, "setpoint_value": 70.0,
               "error_limit": 5.0}),
        json!({"event_type": {"value": 5, "name": "out-of-range"},
               "exceeding_value": 101.5, "status_flags": alarm, "deadband": 1.0,
               "exceeded_limit": 100.0}),
        json!({"event_type": {"value": 6, "name": "complex-event-type"},
               "property_values": [present]}),
        json!({"event_type": {"value": 8, "name": "change-of-life-safety"},
               "new_state": {"value": 2, "name": "alarm"},
               "new_mode": {"value": 1, "name": "on"}, "status_flags": alarm,
               "operation_expected": {"value": 1, "name": "silence"}}),
        json!({"event_type": {"value": 9, "name": "extended"},
               "vendor_id": 555, "extended_event_type": 9, "parameters": 5}),
        json!({"event_type": {"value": 10, "name": "buffer-ready"},
               "buffer_property": {"element": "device-object-property-reference",
                                   "text": "trend-log 1 log-buffer"},
               "previous_notification": 10, "current_notification": 20}),
        json!({"event_type": {"value": 11, "name": "unsigned-range"},
               "exceeding_value": 301, "status_flags": alarm, "exceeded_limit": 300}),
        json!({"event_type": {"value": 13, "name": "access-event"},
               "access_event": {"value": 1, "name": "granted"}, "status_flags": none,
               "access_event_tag": 7,
               "access_event_time": {"element": "date-time",
                                     "text": "2026-10-25 Sun 14:30:05"},
               "access_credential": {"element": "device-object-reference",
                                     "text": "access-credential 1"},
               "authentication_factor": {"element": "authentication-factor",
                                         "text": "simple-number16 class 0 value 0x04d2"}}),
        json!({"event_type": {"value": 14, "name": "double-out-of-range"},
               "exceeding_value": -2.5, "status_flags": alarm, "deadband": 0.5,
               "exceeded_limit": -2.0}),
        json!({"event_type": {"value": 15, "name": "signed-out-of-range"},
               "exceeding_value": -6, "status_flags": alarm, "deadband": 1,
               "exceeded_limit": -5}),
        json!({"event_type": {"value": 16, "name": "unsigned-out-of-range"},
               "exceeding_value": 301, "status_flags": alarm, "deadband": 1,
               "exceeded_limit": 300}),
        json!({"event_type": {"value": 17, "name": "change-of-characterstring"},
               "changed_value": "FIRE", "status_flags": alarm, "alarm_value": "FIRE"}),
        json!({"event_type": {"value": 18, "name": "change-of-status-flags"},
               "present_value": 2, "referenced_flags": fault}),
        json!({"event_type": {"value": 19, "name": "change-of-reliability"},
        "reliability": {"value": 2, "name": "over-range"}, "status_flags": fault,
        "property_values": [{
            "element": "property-value",
            "text": "present-value = 72.5",
            "fields": {
                "property_identifier": {"value": 85, "name": "present-value"},
                "property_array_index": null,
                "value": 72.5,
                "priority": null,
            },
        }]}),
        json!({"event_type": {"value": 21, "name": "change-of-discrete-value"},
               "new_value": 3, "status_flags": none}),
        json!({"event_type": {"value": 22, "name": "change-of-timer"},
               "new_state": {"value": 2, "name": "expired"}, "status_flags": alarm,
               "update_time": {"element": "date-time", "text": "2026-10-25 Sun 14:30:05"},
               "last_state_change": {"value": 4, "name": "running-to-expired"},
               "initial_timeout": 60, "expiration_time": null}),
    ];
    for ((parameters, text), fields) in cases().into_iter().zip(expected) {
        assert_eq!(
            json_of(&EventValues::new(&parameters))["fields"],
            fields,
            "{text}"
        );
    }
}

/// Optional fields left out by the device are left out of the text and
/// null in the data; values that aren't application values stay hex.
#[test]
fn absent_and_undecodable_values() {
    let values = EventValues::new(&NotificationParameters::ChangeOfStatusFlags {
        present_value: None,
        referenced_flags: StatusFlags::FAULT,
    });
    assert_eq!(values.text(), "change-of-status-flags: flags fault");
    assert_eq!(json_of(&values)["fields"]["present_value"], json!(null));

    // A context-tagged datetime in a discrete value isn't an application
    // value: its fields by tag, hex in the data.
    let values = EventValues::new(&NotificationParameters::ChangeOfDiscreteValue {
        new_value: vec![0x0E, 0x0F],
        status_flags: StatusFlags::empty(),
    });
    assert_eq!(
        values.text(),
        "change-of-discrete-value: value [0] (), flags none"
    );
    assert_eq!(
        json_of(&values)["fields"]["new_value"],
        json!({"raw": "0e0f"})
    );
}
