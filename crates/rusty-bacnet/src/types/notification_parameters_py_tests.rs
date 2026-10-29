//! Event notification values as Python mappings (#2073): every event type
//! decodes to a mapping, with no `bytes` for a value the stack decodes.

use std::ffi::CString;

use bacnet_encoding::constructed::{encode_authentication_factor, encode_bacnet_property_value};
use bacnet_types::constructed::{
    BACnetAuthenticationFactor, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetPropertyStates,
};
use bacnet_types::enums::{
    AccessEvent, AuthenticationFactorType, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    ObjectType, PropertyIdentifier, Reliability, TimerState, TimerTransition,
};
use bacnet_types::primitives::{ObjectIdentifier, StatusFlags};
use bytes::BytesMut;
use pyo3::types::{PyList, PyTuple};

use super::*;

fn real(value: f32) -> Vec<u8> {
    [vec![0x44], value.to_be_bytes().to_vec()].concat()
}

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 25,
    day_of_week: 7,
};
const TIME: Time = Time {
    hour: 14,
    minute: 30,
    second: 5,
    hundredths: 0,
};

fn present_value() -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: real(72.5),
        priority: Some(8),
    }
}

/// Every event type, and the mapping it reads as: a Python expression in
/// which `PV(x)` is a REAL `PropertyValue`, `FLAGS(b)` the status flags
/// with bits `b`, `TL1` Trend Log 1, `AC1` Access Credential 1, `PRESENT`
/// and `LOG_BUFFER` those properties, and `SILENCE` that operation.
fn cases() -> Vec<(NotificationParameters, &'static str)> {
    use NotificationParameters as N;
    let alarm = StatusFlags::IN_ALARM;
    let mut values = BytesMut::new();
    encode_bacnet_property_value(&present_value(), &mut values);
    vec![
        (
            N::ChangeOfBitstring {
                referenced_bitstring: (4, vec![0xA0]),
                status_flags: alarm,
            },
            "{'kind': 'change_of_bitstring', 'referenced_bitstring': BitString(4, b'\\xa0'), \
             'status_flags': FLAGS(0x80)}",
        ),
        (
            N::ChangeOfState {
                new_state: BACnetPropertyStates::State(3),
                status_flags: alarm,
            },
            "{'kind': 'change_of_state', 'new_state': (8, 3), 'status_flags': FLAGS(0x80)}",
        ),
        (
            N::ChangeOfValue {
                new_value: ChangeOfValueChoice::ChangedValue(72.5),
                status_flags: StatusFlags::empty(),
            },
            "{'kind': 'change_of_value', 'changed_bits': None, 'changed_value': 72.5, \
             'status_flags': FLAGS(0)}",
        ),
        (
            N::CommandFailure {
                command_value: real(50.0),
                status_flags: StatusFlags::OVERRIDDEN,
                feedback_value: real(30.0),
            },
            "{'kind': 'command_failure', 'command_value': PV(50.0), \
             'status_flags': FLAGS(0x20), 'feedback_value': PV(30.0)}",
        ),
        (
            N::FloatingLimit {
                reference_value: 80.5,
                status_flags: alarm,
                setpoint_value: 70.0,
                error_limit: 5.0,
            },
            "{'kind': 'floating_limit', 'reference_value': 80.5, 'status_flags': FLAGS(0x80), \
             'setpoint_value': 70.0, 'error_limit': 5.0}",
        ),
        (
            N::OutOfRange {
                exceeding_value: 101.5,
                status_flags: alarm,
                deadband: 1.0,
                exceeded_limit: 100.0,
            },
            "{'kind': 'out_of_range', 'exceeding_value': 101.5, 'status_flags': FLAGS(0x80), \
             'deadband': 1.0, 'exceeded_limit': 100.0}",
        ),
        (
            N::ComplexEventType {
                property_values: vec![present_value()],
            },
            "{'kind': 'complex_event_type', 'property_values': [{'property_identifier': \
             PRESENT, 'property_array_index': None, 'value': PV(72.5), 'priority': 8}]}",
        ),
        (
            N::ChangeOfLifeSafety {
                new_state: LifeSafetyState::ALARM,
                new_mode: LifeSafetyMode::ON,
                status_flags: alarm,
                operation_expected: LifeSafetyOperation::SILENCE,
            },
            "{'kind': 'change_of_life_safety', 'new_state': 2, 'new_mode': 1, \
             'status_flags': FLAGS(0x80), 'operation_expected': SILENCE}",
        ),
        (
            N::Extended {
                vendor_id: 555,
                extended_event_type: 9,
                parameters: real(1.5),
            },
            "{'kind': 'extended', 'vendor_id': 555, 'extended_event_type': 9, \
             'parameters': PV(1.5)}",
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
            "{'kind': 'buffer_ready', 'buffer_property': {'object_identifier': TL1, \
             'property_identifier': LOG_BUFFER, 'property_array_index': None, \
             'device_identifier': None}, 'previous_notification': 10, \
             'current_notification': 20}",
        ),
        (
            N::UnsignedRange {
                exceeding_value: 301,
                status_flags: alarm,
                exceeded_limit: 300,
            },
            "{'kind': 'unsigned_range', 'exceeding_value': 301, 'status_flags': FLAGS(0x80), \
             'exceeded_limit': 300}",
        ),
        (
            N::AccessEvent {
                access_event: AccessEvent::GRANTED,
                status_flags: StatusFlags::empty(),
                access_event_tag: 7,
                access_event_time: (DATE, TIME),
                access_credential: BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1)
                        .unwrap(),
                },
                authentication_factor: None,
            },
            "{'kind': 'access_event', 'access_event': 1, 'status_flags': FLAGS(0), \
             'access_event_tag': 7, 'access_event_time': ((2026, 10, 25, 7), (14, 30, 5, 0)), \
             'access_credential': AC1, 'authentication_factor': None}",
        ),
        (
            N::DoubleOutOfRange {
                exceeding_value: -2.5,
                status_flags: alarm,
                deadband: 0.5,
                exceeded_limit: -2.0,
            },
            "{'kind': 'double_out_of_range', 'exceeding_value': -2.5, \
             'status_flags': FLAGS(0x80), 'deadband': 0.5, 'exceeded_limit': -2.0}",
        ),
        (
            N::SignedOutOfRange {
                exceeding_value: -6,
                status_flags: alarm,
                deadband: 1,
                exceeded_limit: -5,
            },
            "{'kind': 'signed_out_of_range', 'exceeding_value': -6, \
             'status_flags': FLAGS(0x80), 'deadband': 1, 'exceeded_limit': -5}",
        ),
        (
            N::UnsignedOutOfRange {
                exceeding_value: 301,
                status_flags: alarm,
                deadband: 1,
                exceeded_limit: 300,
            },
            "{'kind': 'unsigned_out_of_range', 'exceeding_value': 301, \
             'status_flags': FLAGS(0x80), 'deadband': 1, 'exceeded_limit': 300}",
        ),
        (
            N::ChangeOfCharacterstring {
                changed_value: "FIRE".into(),
                status_flags: alarm,
                alarm_value: "FIRE".into(),
            },
            "{'kind': 'change_of_characterstring', 'changed_value': 'FIRE', \
             'status_flags': FLAGS(0x80), 'alarm_value': 'FIRE'}",
        ),
        (
            N::ChangeOfStatusFlags {
                present_value: Some(vec![0x21, 0x02]),
                referenced_flags: StatusFlags::FAULT,
            },
            "{'kind': 'change_of_status_flags', 'present_value': PropertyValue.unsigned(2), \
             'referenced_flags': FLAGS(0x40)}",
        ),
        (
            N::ChangeOfReliability {
                reliability: Reliability::OVER_RANGE,
                status_flags: StatusFlags::FAULT,
                property_values: values.to_vec(),
            },
            "{'kind': 'change_of_reliability', 'reliability': 2, 'status_flags': FLAGS(0x40), \
             'property_values': [{'property_identifier': PRESENT, \
             'property_array_index': None, 'value': PV(72.5), 'priority': 8}]}",
        ),
        (
            N::ChangeOfDiscreteValue {
                new_value: vec![0x21, 0x03],
                status_flags: StatusFlags::empty(),
            },
            "{'kind': 'change_of_discrete_value', 'new_value': PropertyValue.unsigned(3), \
             'status_flags': FLAGS(0)}",
        ),
        (
            N::ChangeOfTimer {
                new_state: TimerState::EXPIRED,
                status_flags: alarm,
                update_time: (DATE, TIME),
                last_state_change: Some(TimerTransition::RUNNING_TO_EXPIRED),
                initial_timeout: Some(60),
                expiration_time: None,
            },
            "{'kind': 'change_of_timer', 'new_state': 2, 'status_flags': FLAGS(0x80), \
             'update_time': ((2026, 10, 25, 7), (14, 30, 5, 0)), 'last_state_change': 4, \
             'initial_timeout': 60, 'expiration_time': None}",
        ),
    ]
}

/// Whether `value`, or anything inside a dict, list or tuple of it, is
/// `bytes`.
fn holds_bytes(value: &Bound<'_, PyAny>) -> bool {
    if value.is_instance_of::<PyBytes>() {
        return true;
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        return dict.values().iter().any(|item| holds_bytes(&item));
    }
    if let Ok(list) = value.cast::<PyList>() {
        return list.iter().any(|item| holds_bytes(&item));
    }
    if let Ok(tuple) = value.cast::<PyTuple>() {
        return tuple.iter().any(|item| holds_bytes(&item));
    }
    false
}

#[test]
fn every_event_type_reads_as_a_mapping_without_bytes() {
    let cases = cases();
    assert_eq!(cases.len(), 20, "every NotificationParameters choice");
    Python::initialize();
    Python::attach(|py| {
        // One namespace for the helpers' globals and the expressions.
        let locals = PyDict::new(py);
        let property_value = py.get_type::<super::super::PyPropertyValue>();
        locals.set_item("PropertyValue", &property_value).unwrap();
        locals
            .set_item("BitString", py.get_type::<PyBitString>())
            .unwrap();
        let property = |inner| PyPropertyIdentifier { inner };
        locals
            .set_item("PRESENT", property(PropertyIdentifier::PRESENT_VALUE))
            .unwrap();
        locals
            .set_item("LOG_BUFFER", property(PropertyIdentifier::LOG_BUFFER))
            .unwrap();
        locals
            .set_item(
                "SILENCE",
                PyLifeSafetyOperation {
                    inner: LifeSafetyOperation::SILENCE,
                },
            )
            .unwrap();
        let object = |object_type, instance| {
            super::super::PyObjectIdentifier::from_rust(
                ObjectIdentifier::new(object_type, instance).unwrap(),
            )
        };
        locals
            .set_item("TL1", object(ObjectType::TREND_LOG, 1))
            .unwrap();
        locals
            .set_item("AC1", object(ObjectType::ACCESS_CREDENTIAL, 1))
            .unwrap();
        let helpers = CString::new(
            "PV = lambda x: PropertyValue.real(x)\n\
             FLAGS = lambda b: BitString(4, bytes([b]))\n",
        )
        .unwrap();
        py.run(&helpers, Some(&locals), None).unwrap();
        for (parameters, expected) in cases {
            let read = notification_parameters_to_py(py, &parameters).unwrap();
            let expected_value = py
                .eval(&CString::new(expected).unwrap(), Some(&locals), None)
                .unwrap();
            assert!(
                read.as_any().eq(&expected_value).unwrap(),
                "{parameters:?}: {read} != {expected_value}"
            );
            assert!(!holds_bytes(read.as_any()), "{parameters:?}: {read}");
        }
    });
}

/// An authentication factor reads as the tuple `report_access_event_local`
/// takes, its value the factor's own octets.
#[test]
fn an_authentication_factor_reads_as_its_tuple() {
    let mut factor = BytesMut::new();
    encode_authentication_factor(
        &mut factor,
        &BACnetAuthenticationFactor {
            format_type: AuthenticationFactorType::SIMPLE_NUMBER16,
            format_class: 0,
            value: vec![0x04, 0xD2],
        },
    );
    Python::initialize();
    Python::attach(|py| {
        let read = authentication_factor(py, &factor).unwrap();
        let expected = py.eval(c"(3, 0, b'\\x04\\xd2')", None, None).unwrap();
        assert!(read.bind(py).eq(&expected).unwrap(), "{read}");
    });
}
