//! Typed reads of the remaining single constructed values (#1499): the
//! stack's own encoders' octets read back typed, keep those octets, give the
//! documented Python forms, and the forms that aren't the production stay
//! with the generic decoder.
use super::*;
use crate::types::read_value::decode_read_value;
use bacnet_encoding::constructed::{
    encode_authentication_factor, encode_event_parameter, encode_shed_level,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::constructed::{
    BACnetAuthenticationFactor, BACnetDeviceObjectPropertyReference, BACnetExtendedPropertyState,
    BACnetPropertyStates, BACnetProprietaryPropertyState, BACnetShedLevel, ChangeOfValueCriteria,
};
use bacnet_types::enums::AuthenticationFactorType;
use bytes::BytesMut;
use std::ffi::CString;

type O = ObjectType;
type P = PropertyIdentifier;

/// Read `octets` as a whole read of `property`, check it is one typed
/// `element` holding exactly those octets, and compare its `.value` with the
/// Python expression `expected`, in which `AV7` is Analog Value 7 and `PV`
/// Present_Value.
fn assert_single((object_type, property): (O, P), octets: &[u8], element: Element, expected: &str) {
    let value = decode_read_value(object_type, property, None, octets).unwrap();
    assert_eq!(value.element, Some(element), "{property:?} {octets:02x?}");
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value.inner).unwrap();
    assert_eq!(encoded.as_ref(), octets);
    Python::initialize();
    Python::attach(|py| {
        let value = Bound::new(py, value).unwrap();
        let tag = value.getattr("tag").unwrap().extract::<String>().unwrap();
        assert_eq!(tag, element.tag());
        let read = value.getattr("value").unwrap();
        let locals = pyo3::types::PyDict::new(py);
        let av7 = ObjectIdentifier::new(O::ANALOG_VALUE, 7).unwrap();
        locals
            .set_item("AV7", super::super::PyObjectIdentifier::from_rust(av7))
            .unwrap();
        let pv = super::super::PyPropertyIdentifier {
            inner: P::PRESENT_VALUE,
        };
        locals.set_item("PV", pv).unwrap();
        locals
            .set_item("BitString", py.get_type::<super::super::PyBitString>())
            .unwrap();
        let expected = CString::new(expected).unwrap();
        let expected = py.eval(&expected, None, Some(&locals)).unwrap();
        assert!(read.eq(&expected).unwrap(), "{read} != {expected}");
    });
}

fn generic(object_type: O, property: P, octets: &[u8]) -> PyPropertyValue {
    let value = decode_read_value(object_type, property, None, octets).unwrap();
    assert_eq!(value.element, None, "{property:?} {octets:02x?}");
    value
}

#[test]
fn shed_levels_read_as_kind_and_value_on_all_three_properties() {
    for property in [
        P::REQUESTED_SHED_LEVEL,
        P::EXPECTED_SHED_LEVEL,
        P::ACTUAL_SHED_LEVEL,
    ] {
        for (level, expected) in [
            (
                BACnetShedLevel::Percent(100),
                "{'kind': 'percent', 'value': 100}",
            ),
            (BACnetShedLevel::Level(0), "{'kind': 'level', 'value': 0}"),
            (
                BACnetShedLevel::Amount(-1.5),
                "{'kind': 'amount', 'value': -1.5}",
            ),
        ] {
            let mut octets = BytesMut::new();
            encode_shed_level(&mut octets, &level);
            assert_single(
                (O::LOAD_CONTROL, property),
                &octets,
                Element::ShedLevel,
                expected,
            );
        }
    }
    // Another object type's property of the same number isn't one.
    generic(O::ANALOG_VALUE, P::REQUESTED_SHED_LEVEL, &[0x09, 0x32]);
}

#[test]
fn authentication_factors_read_as_the_report_triple() {
    let factor = BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::from_raw(6),
        format_class: 70_000,
        value: vec![0xde, 0xad],
    };
    let mut octets = BytesMut::new();
    encode_authentication_factor(&mut octets, &factor);
    for target in [
        (O::ACCESS_POINT, P::ACCESS_EVENT_AUTHENTICATION_FACTOR),
        (O::CREDENTIAL_DATA_INPUT, P::PRESENT_VALUE),
    ] {
        assert_single(
            target,
            &octets,
            Element::AuthenticationFactor,
            r"(6, 70000, b'\xde\xad')",
        );
    }
    // An Access Credential's Present_Value isn't a factor.
    generic(O::ACCESS_CREDENTIAL, P::PRESENT_VALUE, &octets);
}

#[test]
fn single_timestamps_and_the_subordinate_list_reuse_their_forms() {
    let sequence: &[u8] = &[0x19, 0x07];
    for target in [
        (O::ACCESS_POINT, P::ACCESS_EVENT_TIME),
        (O::DEVICE, P::LAST_RESTORE_TIME),
    ] {
        let value = decode_read_value(target.0, target.1, None, sequence).unwrap();
        assert_eq!(value.element, Some(Element::TimeStamp));
        assert_eq!(
            value.inner,
            PropertyValue::ApplicationData(sequence.to_vec())
        );
    }
    let view = (O::STRUCTURED_VIEW, P::SUBORDINATE_LIST);
    let two = [
        0x1c, 0, 0x80, 0, 7, 0x0c, 2, 0, 0, 0x63, 0x1c, 0, 0x80, 0, 7,
    ];
    let value = decode_read_value(view.0, view.1, None, &two).unwrap();
    assert_eq!(value.element, Some(Element::DeviceObjectReference));
    assert!(matches!(&value.inner, PropertyValue::List(items) if items.len() == 2));
    let value = decode_read_value(view.0, view.1, Some(2), &two[5..]).unwrap();
    assert_eq!(value.element, Some(Element::DeviceObjectReference));
}

#[test]
fn the_event_algorithm_inhibit_reference_reads_as_an_object_property_reference() {
    // Binary Value 3's Present_Value, then the same with array index 2.
    let plain: &[u8] = &[0x0c, 0x01, 0x40, 0x00, 0x03, 0x19, 0x55];
    let indexed: &[u8] = &[0x0c, 0x01, 0x40, 0x00, 0x03, 0x19, 0x55, 0x29, 0x02];
    for object_type in [O::ANALOG_INPUT, O::BINARY_VALUE, O::TREND_LOG] {
        for octets in [plain, indexed] {
            let value =
                decode_read_value(object_type, P::EVENT_ALGORITHM_INHIBIT_REF, None, octets)
                    .unwrap();
            assert_eq!(value.element, Some(Element::ObjectPropertyReference));
            assert_eq!(value.inner, PropertyValue::ApplicationData(octets.to_vec()));
        }
    }
    // A device member is not part of this production, so the generic
    // decoder keeps the value.
    let with_device = [plain, &[0x3c, 0x02, 0x00, 0x00, 0x01]].concat();
    let value = decode_read_value(
        O::ANALOG_INPUT,
        P::EVENT_ALGORITHM_INHIBIT_REF,
        None,
        &with_device,
    )
    .unwrap();
    assert_eq!(value.element, None);
}

fn parameter_octets(parameter: &BACnetEventParameter) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_event_parameter(&mut octets, parameter).unwrap();
    octets.to_vec()
}

#[test]
fn event_parameters_read_as_a_mapping_per_algorithm() {
    let target = (O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS);
    let setpoint = BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(O::ANALOG_VALUE, 7).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    );
    let cases = [
        (
            BACnetEventParameter::ChangeOfBitstring {
                time_delay: 2,
                bitmask: (5, vec![0xa0]),
                list_of_values: vec![(5, vec![0x40])],
            },
            "{'kind': 'change_of_bitstring', 'time_delay': 2, \
             'bitmask': BitString(5, b'\\xa0'), \
             'list_of_values': [BitString(5, b'\\x40')]}",
        ),
        (
            BACnetEventParameter::ChangeOfState {
                time_delay: 5,
                list_of_values: vec![
                    BACnetPropertyStates::BooleanValue(false),
                    BACnetPropertyStates::Reliability(7),
                    BACnetPropertyStates::IntegerValue(-3),
                    BACnetPropertyStates::ExtendedValue(
                        BACnetExtendedPropertyState::new(300, 12).unwrap(),
                    ),
                    BACnetPropertyStates::Other(
                        BACnetProprietaryPropertyState::primitive(70, vec![0x01, 0x02]).unwrap(),
                    ),
                ],
            },
            "{'kind': 'change_of_state', 'time_delay': 5, 'list_of_values': \
             [(0, False), (7, 7), (41, -3), (300, 12), (70, b'\\x01\\x02')]}",
        ),
        (
            BACnetEventParameter::ChangeOfValue {
                time_delay: 0,
                criteria: ChangeOfValueCriteria::Bitmask {
                    unused_bits: 0,
                    data: vec![0xff],
                },
            },
            "{'kind': 'change_of_value', 'time_delay': 0, \
             'bitmask': BitString(0, b'\\xff'), \
             'referenced_property_increment': None}",
        ),
        (
            BACnetEventParameter::FloatingLimit {
                time_delay: 10,
                setpoint_reference: setpoint,
                low_diff_limit: 1.0,
                high_diff_limit: 2.0,
                deadband: 0.5,
            },
            "{'kind': 'floating_limit', 'time_delay': 10, 'setpoint_reference': \
             {'object_identifier': AV7, 'property_identifier': PV, \
              'property_array_index': None, 'device_identifier': None}, \
             'low_diff_limit': 1.0, 'high_diff_limit': 2.0, 'deadband': 0.5}",
        ),
        (
            BACnetEventParameter::Extended {
                vendor_id: 555,
                extended_event_type: 9,
                parameters: vec![0x21, 0x05],
            },
            "{'kind': 'extended', 'vendor_id': 555, 'extended_event_type': 9, \
             'parameters': b'\\x21\\x05'}",
        ),
        (BACnetEventParameter::None, "{'kind': 'none'}"),
        (
            BACnetEventParameter::Opaque {
                tag: 30,
                data: vec![0x09, 0x05, 0x1e, 0x0c, 0, 0x80, 0, 7, 0x19, 0x55, 0x1f],
            },
            "{'kind': 'other', 'tag': 30, \
             'data': b'\\x09\\x05\\x1e\\x0c\\x00\\x80\\x00\\x07\\x19\\x55\\x1f'}",
        ),
    ];
    for (parameter, expected) in cases {
        assert_single(
            target,
            &parameter_octets(&parameter),
            Element::EventParameter,
            expected,
        );
    }
}

/// The algorithms #2090 modelled read as mappings too, and keep their
/// octets.
#[test]
fn newly_modelled_event_parameters_read_as_mappings() {
    use BACnetEventParameter as EP;
    use bacnet_types::enums::{AccessEvent, LifeSafetyState, TimerState};
    use bacnet_types::primitives::StatusFlags;
    let target = (O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS);
    let av7 = || {
        BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(O::ANALOG_VALUE, 7).unwrap(),
            P::PRESENT_VALUE.to_raw(),
        )
    };
    let av7_py = "{'object_identifier': AV7, 'property_identifier': PV, \
                  'property_array_index': None, 'device_identifier': None}";
    let cases = [
        (
            EP::CommandFailure {
                time_delay: 4,
                feedback_property_reference: av7(),
            },
            format!(
                "{{'kind': 'command_failure', 'time_delay': 4, \
                 'feedback_property_reference': {av7_py}}}"
            ),
        ),
        (
            EP::ChangeOfLifeSafety {
                time_delay: 10,
                list_of_life_safety_alarm_values: vec![LifeSafetyState::ALARM],
                list_of_alarm_values: vec![LifeSafetyState::PRE_ALARM, LifeSafetyState::FAULT],
                mode_property_reference: av7(),
            },
            format!(
                "{{'kind': 'change_of_life_safety', 'time_delay': 10, \
                 'list_of_life_safety_alarm_values': [2], 'list_of_alarm_values': [1, 3], \
                 'mode_property_reference': {av7_py}}}"
            ),
        ),
        (
            EP::BufferReady {
                notification_threshold: 50,
                previous_notification_count: 7,
            },
            "{'kind': 'buffer_ready', 'notification_threshold': 50, \
             'previous_notification_count': 7}"
                .into(),
        ),
        (
            EP::UnsignedRange {
                time_delay: 30,
                low_limit: 10,
                high_limit: 300,
            },
            "{'kind': 'unsigned_range', 'time_delay': 30, 'low_limit': 10, 'high_limit': 300}"
                .into(),
        ),
        (
            EP::AccessEvent {
                list_of_access_events: vec![AccessEvent::GRANTED],
                access_event_time_reference: av7(),
            },
            format!(
                "{{'kind': 'access_event', 'list_of_access_events': [1], \
                 'access_event_time_reference': {av7_py}}}"
            ),
        ),
        (
            EP::DoubleOutOfRange {
                time_delay: 0,
                low_limit: -1.5,
                high_limit: 99.25,
                deadband: 0.5,
            },
            "{'kind': 'double_out_of_range', 'time_delay': 0, 'low_limit': -1.5, \
             'high_limit': 99.25, 'deadband': 0.5}"
                .into(),
        ),
        (
            EP::SignedOutOfRange {
                time_delay: 30,
                low_limit: -5,
                high_limit: 40,
                deadband: 2,
            },
            "{'kind': 'signed_out_of_range', 'time_delay': 30, 'low_limit': -5, \
             'high_limit': 40, 'deadband': 2}"
                .into(),
        ),
        (
            EP::UnsignedOutOfRange {
                time_delay: 5,
                low_limit: 10,
                high_limit: 300,
                deadband: 1,
            },
            "{'kind': 'unsigned_out_of_range', 'time_delay': 5, 'low_limit': 10, \
             'high_limit': 300, 'deadband': 1}"
                .into(),
        ),
        (
            EP::ChangeOfCharacterstring {
                time_delay: 2,
                list_of_alarm_values: vec!["FIRE".into(), String::new()],
            },
            "{'kind': 'change_of_characterstring', 'time_delay': 2, \
             'list_of_alarm_values': ['FIRE', '']}"
                .into(),
        ),
        (
            EP::ChangeOfStatusFlags {
                time_delay: 1,
                selected_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
            "{'kind': 'change_of_status_flags', 'time_delay': 1, \
             'selected_flags': BitString(4, b'\\xc0')}"
                .into(),
        ),
        (
            EP::ChangeOfDiscreteValue { time_delay: 9 },
            "{'kind': 'change_of_discrete_value', 'time_delay': 9}".into(),
        ),
        (
            EP::ChangeOfTimer {
                time_delay: 3,
                alarm_values: vec![TimerState::EXPIRED],
                update_time_reference: av7(),
            },
            format!(
                "{{'kind': 'change_of_timer', 'time_delay': 3, 'alarm_values': [2], \
                 'update_time_reference': {av7_py}}}"
            ),
        ),
    ];
    for (parameter, expected) in cases {
        assert_single(
            target,
            &parameter_octets(&parameter),
            Element::EventParameter,
            &expected,
        );
    }
}

#[test]
fn event_parameter_forms_that_are_not_the_choice_stay_generic() {
    let (object, property) = (O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS);
    // The enrollment's unset form, an application octet string.
    let value = generic(object, property, &[0x60]);
    assert_eq!(value.inner, PropertyValue::OctetString(Vec::new()));
    // The legacy stored wrapper uses a reserved tag, so the read's framing
    // check refuses it before any typed decode.
    let legacy = [0xfe, 0xff, 0x21, 0x01, 0xff, 0xff];
    assert!(decode_read_value(object, property, None, &legacy).is_err());
    // Two values back to back are not one.
    let none = parameter_octets(&BACnetEventParameter::None);
    let twice = [none.as_slice(), none.as_slice()].concat();
    let value = generic(object, property, &twice);
    assert_eq!(value.inner, PropertyValue::ApplicationData(twice));
}
