//! Every Event_Parameters algorithm, as text and as typed data (#2090).

use bacnet_encoding::constructed::encode_event_parameter;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetPropertyStates, ChangeOfValueCriteria,
};
use bacnet_types::enums::{AccessEvent, LifeSafetyState, ObjectType, TimerState};
use bacnet_types::primitives::{ObjectIdentifier, StatusFlags};
use bytes::BytesMut;
use serde_json::json;

use super::*;
use crate::format::{Floats, StateTexts};

type O = ObjectType;
type P = PropertyIdentifier;

fn reference(
    object_type: ObjectType,
    instance: u32,
    property: P,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(object_type, instance).unwrap(),
        property.to_raw(),
    )
}

fn octets(parameter: &BACnetEventParameter) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, parameter).unwrap();
    buf.to_vec()
}

/// The value as the CLI shows it, its text and its JSON.
fn shown(parameter: &BACnetEventParameter) -> (String, serde_json::Value) {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::EVENT_ENROLLMENT, &texts, Floats::Exact).whole_lists();
    let data = octets(parameter);
    let text = format.value(P::EVENT_PARAMETERS, &data);
    let json = serde_json::to_value(format.data(P::EVENT_PARAMETERS, &data)).unwrap();
    assert_eq!(json["element"], "event-parameter");
    assert_eq!(json["text"], text.as_str(), "the JSON carries the text");
    (text, json)
}

fn algorithm(value: u32, name: &str) -> serde_json::Value {
    json!({"value": value, "name": name})
}

fn reference_json(text: &str) -> serde_json::Value {
    json!({"element": "device-object-property-reference", "text": text})
}

/// Each algorithm: its text, and its fields by name with the algorithm
/// first.
#[test]
fn every_algorithm_by_its_fields() {
    use BACnetEventParameter as EP;
    let cases = [
        (
            EP::ChangeOfBitstring {
                time_delay: 1,
                bitmask: (4, vec![0xC0]),
                list_of_values: vec![(4, vec![0x80])],
            },
            "change-of-bitstring: delay 1 s, mask bits(1100), alarm values [bits(1000)]",
            json!({
                "algorithm": algorithm(0, "change-of-bitstring"),
                "time_delay": 1,
                "bitmask": {"bits": "1100"},
                "list_of_bitstring_values": [{"bits": "1000"}],
            }),
        ),
        (
            EP::ChangeOfState {
                time_delay: 0,
                list_of_values: vec![BACnetPropertyStates::State(3)],
            },
            "change-of-state: delay 0 s, alarm values [high-limit]",
            json!({
                "algorithm": algorithm(1, "change-of-state"),
                "time_delay": 0,
                "list_of_values": [{"element": "property-state", "text": "high-limit"}],
            }),
        ),
        (
            EP::ChangeOfValue {
                time_delay: 2,
                criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(0.5),
            },
            "change-of-value: delay 2 s, increment 0.5",
            json!({
                "algorithm": algorithm(2, "change-of-value"),
                "time_delay": 2,
                "referenced_property_increment": 0.5,
            }),
        ),
        (
            EP::CommandFailure {
                time_delay: 4,
                feedback_property_reference: reference(O::BINARY_INPUT, 3, P::PRESENT_VALUE),
            },
            "command-failure: delay 4 s, feedback binary-input 3 present-value",
            json!({
                "algorithm": algorithm(3, "command-failure"),
                "time_delay": 4,
                "feedback_property_reference": reference_json("binary-input 3 present-value"),
            }),
        ),
        (
            EP::FloatingLimit {
                time_delay: 2,
                setpoint_reference: reference(O::ANALOG_VALUE, 1, P::PRESENT_VALUE),
                low_diff_limit: 2.0,
                high_diff_limit: 3.0,
                deadband: 0.5,
            },
            "floating-limit: delay 2 s, setpoint analog-value 1 present-value, \
             low diff 2, high diff 3, deadband 0.5",
            json!({
                "algorithm": algorithm(4, "floating-limit"),
                "time_delay": 2,
                "setpoint_reference": reference_json("analog-value 1 present-value"),
                "low_diff_limit": 2.0,
                "high_diff_limit": 3.0,
                "deadband": 0.5,
            }),
        ),
        (
            EP::OutOfRange {
                time_delay: 5,
                low_limit: 10.0,
                high_limit: 90.5,
                deadband: 1.0,
            },
            "out-of-range: delay 5 s, low 10, high 90.5, deadband 1",
            json!({
                "algorithm": algorithm(5, "out-of-range"),
                "time_delay": 5,
                "low_limit": 10.0,
                "high_limit": 90.5,
                "deadband": 1.0,
            }),
        ),
        (
            EP::ChangeOfLifeSafety {
                time_delay: 10,
                list_of_life_safety_alarm_values: vec![LifeSafetyState::ALARM],
                list_of_alarm_values: vec![
                    LifeSafetyState::PRE_ALARM,
                    LifeSafetyState::from_raw(300),
                ],
                mode_property_reference: reference(O::LIFE_SAFETY_ZONE, 1, P::MODE),
            },
            "change-of-life-safety: delay 10 s, life-safety alarm values [alarm], \
             alarm values [pre-alarm, 300], mode life-safety-zone 1 mode",
            json!({
                "algorithm": algorithm(8, "change-of-life-safety"),
                "time_delay": 10,
                "list_of_life_safety_alarm_values": [{"value": 2, "name": "alarm"}],
                "list_of_alarm_values": [{"value": 1, "name": "pre-alarm"}, {"value": 300}],
                "mode_property_reference": reference_json("life-safety-zone 1 mode"),
            }),
        ),
        (
            EP::Extended {
                vendor_id: 555,
                extended_event_type: 9,
                parameters: vec![0x21, 0x05],
            },
            "extended: vendor 555 type 9, parameters (5)",
            json!({
                "algorithm": algorithm(9, "extended"),
                "vendor_id": 555,
                "extended_event_type": 9,
                "parameters": 5,
            }),
        ),
        (
            EP::BufferReady {
                notification_threshold: 50,
                previous_notification_count: 7,
            },
            "buffer-ready: threshold 50, previous count 7",
            json!({
                "algorithm": algorithm(10, "buffer-ready"),
                "notification_threshold": 50,
                "previous_notification_count": 7,
            }),
        ),
        (
            EP::UnsignedRange {
                time_delay: 30,
                low_limit: 10,
                high_limit: 300,
            },
            "unsigned-range: delay 30 s, low 10, high 300",
            json!({
                "algorithm": algorithm(11, "unsigned-range"),
                "time_delay": 30,
                "low_limit": 10,
                "high_limit": 300,
            }),
        ),
        (
            EP::AccessEvent {
                list_of_access_events: vec![
                    AccessEvent::GRANTED,
                    AccessEvent::DENIED_UNKNOWN_CREDENTIAL,
                ],
                access_event_time_reference: reference(O::ACCESS_POINT, 2, P::ACCESS_EVENT_TIME),
            },
            "access-event: access events [granted, denied-unknown-credential], \
             time reference access-point 2 access-event-time",
            json!({
                "algorithm": algorithm(13, "access-event"),
                "list_of_access_events": [
                    {"value": 1, "name": "granted"},
                    {"value": 129, "name": "denied-unknown-credential"},
                ],
                "access_event_time_reference":
                    reference_json("access-point 2 access-event-time"),
            }),
        ),
        (
            EP::DoubleOutOfRange {
                time_delay: 0,
                low_limit: -1.5,
                high_limit: 99.25,
                deadband: 0.5,
            },
            "double-out-of-range: delay 0 s, low -1.5, high 99.25, deadband 0.5",
            json!({
                "algorithm": algorithm(14, "double-out-of-range"),
                "time_delay": 0,
                "low_limit": -1.5,
                "high_limit": 99.25,
                "deadband": 0.5,
            }),
        ),
        (
            EP::SignedOutOfRange {
                time_delay: 30,
                low_limit: -5,
                high_limit: 40,
                deadband: 2,
            },
            "signed-out-of-range: delay 30 s, low -5, high 40, deadband 2",
            json!({
                "algorithm": algorithm(15, "signed-out-of-range"),
                "time_delay": 30,
                "low_limit": -5,
                "high_limit": 40,
                "deadband": 2,
            }),
        ),
        (
            EP::UnsignedOutOfRange {
                time_delay: 5,
                low_limit: 10,
                high_limit: 300,
                deadband: 1,
            },
            "unsigned-out-of-range: delay 5 s, low 10, high 300, deadband 1",
            json!({
                "algorithm": algorithm(16, "unsigned-out-of-range"),
                "time_delay": 5,
                "low_limit": 10,
                "high_limit": 300,
                "deadband": 1,
            }),
        ),
        (
            EP::ChangeOfCharacterstring {
                time_delay: 2,
                list_of_alarm_values: vec!["FIRE".into(), String::new()],
            },
            "change-of-characterstring: delay 2 s, alarm values [\"FIRE\", \"\"]",
            json!({
                "algorithm": algorithm(17, "change-of-characterstring"),
                "time_delay": 2,
                "list_of_alarm_values": ["FIRE", ""],
            }),
        ),
        (
            EP::ChangeOfStatusFlags {
                time_delay: 1,
                selected_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
            "change-of-status-flags: delay 1 s, flags in-alarm | fault",
            json!({
                "algorithm": algorithm(18, "change-of-status-flags"),
                "time_delay": 1,
                "selected_flags": {"bits": "1100", "names": ["in-alarm", "fault"]},
            }),
        ),
        (
            EP::None,
            "none",
            json!({"algorithm": algorithm(20, "none")}),
        ),
        (
            EP::ChangeOfDiscreteValue { time_delay: 9 },
            "change-of-discrete-value: delay 9 s",
            json!({
                "algorithm": algorithm(21, "change-of-discrete-value"),
                "time_delay": 9,
            }),
        ),
        (
            EP::ChangeOfTimer {
                time_delay: 3,
                alarm_values: vec![TimerState::EXPIRED],
                update_time_reference: reference(O::TIMER, 1, P::UPDATE_TIME),
            },
            "change-of-timer: delay 3 s, alarm values [expired], update time timer 1 update-time",
            json!({
                "algorithm": algorithm(22, "change-of-timer"),
                "time_delay": 3,
                "alarm_values": [{"value": 2, "name": "expired"}],
                "update_time_reference": reference_json("timer 1 update-time"),
            }),
        ),
        (
            EP::Opaque {
                tag: 30,
                data: vec![0x09, 0x1E],
            },
            "30: [0] [raw 1e]",
            json!({"algorithm": {"value": 30}, "data": {"raw": "091e"}}),
        ),
    ];
    for (parameter, text, fields) in cases {
        let (shown_text, json) = shown(&parameter);
        assert_eq!(shown_text, text);
        assert_eq!(json["fields"], fields, "{text}");
    }
}

/// The fields keep their wire order, and the algorithm comes first.
#[test]
fn fields_keep_their_order() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::EVENT_ENROLLMENT, &texts, Floats::Exact);
    let data = octets(&BACnetEventParameter::UnsignedOutOfRange {
        time_delay: 5,
        low_limit: 10,
        high_limit: 300,
        deadband: 1,
    });
    // Written straight from the datum: a JSON value would sort the keys.
    let text = serde_json::to_string(&format.data(P::EVENT_PARAMETERS, &data)).unwrap();
    assert_eq!(
        text,
        r#"{"element":"event-parameter","text":"unsigned-out-of-range: delay 5 s, low 10, high 300, deadband 1","fields":{"algorithm":{"value":16,"name":"unsigned-out-of-range"},"time_delay":5,"low_limit":10,"high_limit":300,"deadband":1}}"#
    );
}
