//! The renderers of `recipients` and `commands`, one value each, the
//! element-table walk, and the typed data the CLI's JSON carries (#2079,
//! #2078).

use bacnet_encoding::constructed::{
    ConstructedElement, encode_access_rule, encode_action_list, encode_authentication_factor,
    encode_authentication_factor_format, encode_authentication_policy, encode_cov_subscription,
    encode_destination, encode_event_parameter, encode_port_permission, encode_prescale,
    encode_property_access_result, encode_read_access_specification, encode_recipient,
    encode_scale, encode_shed_level, encode_stage_limit_value,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::rpm::{ReadAccessResult, ReadResultElement};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    AccessResult, BACnetAccessRule, BACnetActionCommand, BACnetActionList, BACnetAddress,
    BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat, BACnetAuthenticationPolicy,
    BACnetAuthenticationPolicyEntry, BACnetCOVSubscription, BACnetDestination,
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetEventParameter,
    BACnetObjectPropertyReference, BACnetPortPermission, BACnetPrescale,
    BACnetPropertyAccessResult, BACnetPropertyStates, BACnetRecipient, BACnetRecipientProcess,
    BACnetScale, BACnetShedLevel, BACnetStageLimitValue, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::enums::{AuthenticationFactorType, ErrorClass, ErrorCode};
use bacnet_types::primitives::{ObjectIdentifier, Time};
use bytes::BytesMut;

use super::*;

type O = ObjectType;
type P = PropertyIdentifier;

fn text(object_type: ObjectType, property: PropertyIdentifier, data: &[u8]) -> String {
    let texts = StateTexts::default();
    ValueFormat::new(object_type, &texts, Floats::Display).value(property, data)
}

fn data(object_type: ObjectType, property: PropertyIdentifier, octets: &[u8]) -> Datum {
    let texts = StateTexts::default();
    ValueFormat::new(object_type, &texts, Floats::Exact).data(property, octets)
}

fn bytes(write: impl FnOnce(&mut BytesMut)) -> Vec<u8> {
    let mut buf = BytesMut::new();
    write(&mut buf);
    buf.to_vec()
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn hm(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn reference(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(object, property.to_raw())
}

#[test]
fn every_element_in_the_table_has_a_renderer() {
    for element in ConstructedElement::ALL {
        assert!(
            constructed::renderer(element).is_some(),
            "{element:?} falls back to the generic view"
        );
    }
}

#[test]
fn recipient_lists_name_days_times_and_transitions() {
    let office = BACnetDestination {
        valid_days: DaysOfWeek::from_bits_truncate(0b001_1111),
        from_time: hm(8, 0),
        to_time: hm(17, 0),
        recipient: BACnetRecipient::Device(oid(O::DEVICE, 1234)),
        process_identifier: 7,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    };
    let anyone = BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: hm(0, 0),
        to_time: hm(23, 59),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 5,
            mac_address: MacAddr::from_slice(&[0x0a]),
        }),
        process_identifier: 0,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::empty(),
    };
    let list = bytes(|buf| {
        encode_destination(buf, &office).unwrap();
        encode_destination(buf, &anyone).unwrap();
    });
    assert_eq!(
        text(O::NOTIFICATION_CLASS, P::RECIPIENT_LIST, &list),
        "[device 1234 process 7: Mon-Fri 08:00-17:00, confirmed, to-offnormal | to-normal; \
         network 5 address 0x0a process 0: every day 00:00-23:59, unconfirmed, no transitions]"
    );
    let audit =
        bytes(|buf| encode_recipient(buf, &BACnetRecipient::Device(oid(O::DEVICE, 9))).unwrap());
    assert_eq!(
        text(O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT, &audit),
        "device 9"
    );
}

#[test]
fn event_parameters_name_the_algorithm_and_its_parameters() {
    let cases = [
        (
            BACnetEventParameter::OutOfRange {
                time_delay: 5,
                low_limit: 10.0,
                high_limit: 90.5,
                deadband: 1.0,
            },
            "out-of-range: delay 5 s, low 10, high 90.5, deadband 1",
        ),
        (
            BACnetEventParameter::ChangeOfState {
                time_delay: 0,
                list_of_values: vec![
                    BACnetPropertyStates::State(3),
                    BACnetPropertyStates::BinaryValue(1),
                ],
            },
            "change-of-state: delay 0 s, alarm values [high-limit, active]",
        ),
        (
            BACnetEventParameter::FloatingLimit {
                time_delay: 2,
                setpoint_reference: reference(oid(O::ANALOG_VALUE, 1), P::PRESENT_VALUE),
                low_diff_limit: 2.0,
                high_diff_limit: 3.0,
                deadband: 0.5,
            },
            "floating-limit: delay 2 s, setpoint analog-value 1 present-value, \
             low diff 2, high diff 3, deadband 0.5",
        ),
        (
            BACnetEventParameter::ChangeOfBitstring {
                time_delay: 1,
                bitmask: (4, vec![0xC0]),
                list_of_values: vec![(4, vec![0x80])],
            },
            "change-of-bitstring: delay 1 s, mask bits(1100), alarm values [bits(1000)]",
        ),
    ];
    for (parameter, shown) in cases {
        let octets = bytes(|buf| encode_event_parameter(buf, &parameter).unwrap());
        assert_eq!(
            text(O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS, &octets),
            shown
        );
    }
}

#[test]
fn cov_subscriptions_and_address_bindings() {
    let subscription = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(oid(O::DEVICE, 1234)),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            oid(O::ANALOG_INPUT, 1),
            P::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: false,
        time_remaining: 120,
        cov_increment: Some(0.5),
    };
    let octets = bytes(|buf| encode_cov_subscription(buf, &subscription).unwrap());
    assert_eq!(
        text(O::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS, &octets),
        "[device 1234 process 7: analog-input 1 present-value, unconfirmed, 120 s left, \
         increment 0.5]"
    );
    let binding = bytes(|buf| {
        for value in [
            PropertyValue::ObjectIdentifier(oid(O::DEVICE, 55)),
            PropertyValue::Unsigned(0),
            PropertyValue::OctetString(vec![10, 0, 0, 1, 0xBA, 0xC0]),
        ] {
            encode_property_value(buf, &value).unwrap();
        }
    });
    assert_eq!(
        text(O::DEVICE, P::DEVICE_ADDRESS_BINDING, &binding),
        "[device 55 at network 0 address 0x0a000001bac0]"
    );
}

#[test]
fn command_actions_name_each_write_with_the_target_states() {
    let list = BACnetActionList {
        commands: vec![
            BACnetActionCommand {
                device_identifier: None,
                object_identifier: oid(O::BINARY_OUTPUT, 2),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Enumerated(1),
                priority: Some(8),
                post_delay: Some(5),
                quit_on_failure: true,
                write_successful: false,
            },
            BACnetActionCommand {
                device_identifier: Some(oid(O::DEVICE, 9)),
                object_identifier: oid(O::ANALOG_OUTPUT, 1),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Real(50.0),
                priority: None,
                post_delay: None,
                quit_on_failure: false,
                write_successful: true,
            },
        ],
    };
    let octets = bytes(|buf| {
        encode_action_list(buf, &list).unwrap();
        encode_action_list(buf, &BACnetActionList::default()).unwrap();
    });
    assert_eq!(
        text(O::COMMAND, P::ACTION, &octets),
        "[(binary-output 2 present-value = active @8, then wait 5 s, stop on failure; \
         analog-output 1 present-value = 50 on device 9); no writes]"
    );
}

#[test]
fn group_members_and_results() {
    let spec = ReadAccessSpecification {
        object_identifier: oid(O::ANALOG_INPUT, 1),
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: P::PRIORITY_ARRAY,
                property_array_index: Some(3),
            },
        ],
    };
    let members = bytes(|buf| encode_read_access_specification(buf, &spec));
    assert_eq!(
        text(O::GROUP, P::LIST_OF_GROUP_MEMBERS, &members),
        "[analog-input 1: present-value, priority-array[3]]"
    );
    let value = bytes(|buf| {
        encode_property_value(buf, &PropertyValue::Enumerated(2)).unwrap();
    });
    let result = ReadAccessResult {
        object_identifier: oid(O::ANALOG_INPUT, 1),
        list_of_results: vec![
            ReadResultElement {
                property_identifier: P::EVENT_STATE,
                property_array_index: None,
                property_value: Some(value),
                error: None,
            },
            ReadResultElement {
                property_identifier: P::UNITS,
                property_array_index: None,
                property_value: None,
                error: Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
            },
        ],
    };
    let present = bytes(|buf| result.encode(buf));
    assert_eq!(
        text(O::GROUP, P::PRESENT_VALUE, &present),
        "[analog-input 1: event-state = offnormal, units = error property:unknown-property]"
    );
    let access = |access_result| BACnetPropertyAccessResult {
        reference: reference(oid(O::BINARY_INPUT, 4), P::PRESENT_VALUE),
        access_result,
    };
    let global = bytes(|buf| {
        encode_property_access_result(
            buf,
            &access(AccessResult::Value(PropertyValue::Enumerated(0))),
        )
        .unwrap();
        encode_property_access_result(buf, &access(AccessResult::NOT_INITIALIZED)).unwrap();
    });
    assert_eq!(
        text(O::GLOBAL_GROUP, P::PRESENT_VALUE, &global),
        "[binary-input 4 present-value = inactive; \
         binary-input 4 present-value = error property:value-not-initialized]"
    );
}

#[test]
fn accumulator_load_control_and_staging_values() {
    let scale = |s| bytes(|buf| encode_scale(buf, &s));
    assert_eq!(
        text(
            O::ACCUMULATOR,
            P::SCALE,
            &scale(BACnetScale::FloatScale(0.5))
        ),
        "factor 0.5"
    );
    assert_eq!(
        text(
            O::ACCUMULATOR,
            P::SCALE,
            &scale(BACnetScale::IntegerScale(-2))
        ),
        "factor 10^-2"
    );
    let prescale = bytes(|buf| {
        encode_prescale(
            buf,
            &BACnetPrescale {
                multiplier: 1,
                modulo_divide: 10,
            },
        )
    });
    assert_eq!(
        text(O::ACCUMULATOR, P::PRESCALE, &prescale),
        "multiplier 1, modulo divide 10"
    );
    for (level, shown) in [
        (BACnetShedLevel::Percent(80), "80 %"),
        (BACnetShedLevel::Level(2), "level 2"),
        (BACnetShedLevel::Amount(1.5), "reduce 1.5 kW"),
    ] {
        let octets = bytes(|buf| encode_shed_level(buf, &level));
        assert_eq!(text(O::LOAD_CONTROL, P::ACTUAL_SHED_LEVEL, &octets), shown);
    }
    let stages = bytes(|buf| {
        encode_stage_limit_value(
            buf,
            &BACnetStageLimitValue {
                limit: 50.0,
                values: vec![true, false, true],
                deadband: 2.0,
            },
        );
    });
    assert_eq!(
        text(O::STAGING, P::STAGES, &stages),
        "[limit 50, deadband 2, targets 101]"
    );
}

#[test]
fn access_control_values() {
    let ports = bytes(|buf| {
        encode_port_permission(
            buf,
            &BACnetPortPermission {
                port_id: 1,
                enabled: true,
            },
        );
        encode_port_permission(
            buf,
            &BACnetPortPermission {
                port_id: 2,
                enabled: false,
            },
        );
    });
    assert_eq!(
        text(O::NOTIFICATION_FORWARDER, P::PORT_FILTER, &ports),
        "[port 1 enabled, port 2 disabled]"
    );
    let formats = bytes(|buf| {
        encode_authentication_factor_format(
            buf,
            &BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::SIMPLE_NUMBER16),
        );
        encode_authentication_factor_format(buf, &BACnetAuthenticationFactorFormat::custom(7, 2));
    });
    assert_eq!(
        text(O::CREDENTIAL_DATA_INPUT, P::SUPPORTED_FORMATS, &formats),
        "[simple-number16, custom (vendor 7 format 2)]"
    );
    let factor = bytes(|buf| {
        encode_authentication_factor(
            buf,
            &BACnetAuthenticationFactor {
                format_type: AuthenticationFactorType::SIMPLE_NUMBER16,
                format_class: 0,
                value: vec![0x04, 0xD2],
            },
        )
    });
    assert_eq!(
        text(O::CREDENTIAL_DATA_INPUT, P::PRESENT_VALUE, &factor),
        "simple-number16 class 0 value 0x04d2"
    );
    let reader = |instance| BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: oid(O::CREDENTIAL_DATA_INPUT, instance),
    };
    let policy = bytes(|buf| {
        encode_authentication_policy(
            buf,
            &BACnetAuthenticationPolicy {
                policy: vec![
                    BACnetAuthenticationPolicyEntry {
                        credential_data_input: reader(1),
                        index: 1,
                    },
                    BACnetAuthenticationPolicyEntry {
                        credential_data_input: reader(2),
                        index: 2,
                    },
                ],
                order_enforced: true,
                timeout: 30,
            },
        );
        encode_authentication_policy(buf, &BACnetAuthenticationPolicy::default());
    });
    assert_eq!(
        text(O::ACCESS_POINT, P::AUTHENTICATION_POLICY_LIST, &policy),
        "[credential-data-input 1 step 1, credential-data-input 2 step 2, in order, \
         timeout 30 s; no factors, any order, no timeout]"
    );
    let rules = bytes(|buf| {
        encode_access_rule(buf, &BACnetAccessRule::new(None, None, true));
        encode_access_rule(
            buf,
            &BACnetAccessRule::new(
                Some(reference(oid(O::SCHEDULE, 1), P::PRESENT_VALUE)),
                Some(BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: oid(O::ACCESS_POINT, 3),
                }),
                false,
            ),
        );
    });
    assert_eq!(
        text(O::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES, &rules),
        "[always at every point, enabled; \
         when schedule 1 present-value at access-point 3, disabled]"
    );
}

#[test]
fn data_carries_numbers_with_names() {
    let encoded = |value: PropertyValue| bytes(|buf| encode_property_value(buf, &value).unwrap());
    assert_eq!(
        data(
            O::ANALOG_INPUT,
            P::EVENT_STATE,
            &encoded(PropertyValue::Enumerated(2))
        ),
        Datum::Enumerated {
            value: 2,
            name: Some("offnormal".into())
        }
    );
    assert_eq!(
        data(
            O::BINARY_INPUT,
            P::PRESENT_VALUE,
            &encoded(PropertyValue::Enumerated(1))
        ),
        Datum::Enumerated {
            value: 1,
            name: Some("active".into())
        }
    );
    assert_eq!(
        data(
            O::ANALOG_INPUT,
            P::DESCRIPTION,
            &encoded(PropertyValue::Enumerated(7))
        ),
        Datum::Enumerated {
            value: 7,
            name: None
        }
    );
    let flags = encoded(PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0b0100_0000],
    });
    assert_eq!(
        data(O::ANALOG_INPUT, P::STATUS_FLAGS, &flags),
        Datum::Bits {
            bits: "0100".into(),
            names: Some(vec!["fault".into()])
        }
    );
    let none = encoded(PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0],
    });
    assert_eq!(
        data(O::ANALOG_INPUT, P::STATUS_FLAGS, &none),
        Datum::Bits {
            bits: "0000".into(),
            names: Some(vec![])
        }
    );
    assert_eq!(
        data(O::ANALOG_INPUT, P::DESCRIPTION, &none),
        Datum::Bits {
            bits: "0000".into(),
            names: None
        }
    );
    assert_eq!(
        data(
            O::ANALOG_INPUT,
            P::PRESENT_VALUE,
            &encoded(PropertyValue::Real(0.1))
        ),
        Datum::Real(0.1)
    );
    assert_eq!(
        data(
            O::ANALOG_INPUT,
            P::PRESENT_VALUE,
            &encoded(PropertyValue::Real(f32::NAN))
        ),
        Datum::NonFinite {
            float: "NaN".into()
        }
    );
    assert_eq!(
        data(
            O::DEVICE,
            P::OBJECT_IDENTIFIER,
            &encoded(PropertyValue::ObjectIdentifier(oid(O::ANALOG_INPUT, 3)))
        ),
        Datum::Object {
            object_type: "analog-input".into(),
            type_number: 0,
            instance: 3
        }
    );
    assert_eq!(
        data(O::ANALOG_INPUT, P::DESCRIPTION, &[0xFF]),
        Datum::Raw { raw: "ff".into() }
    );
    let audit =
        bytes(|buf| encode_recipient(buf, &BACnetRecipient::Device(oid(O::DEVICE, 9))).unwrap());
    assert_eq!(
        data(O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT, &audit),
        Datum::Constructed {
            element: "recipient".into(),
            text: "device 9".into(),
            fields: None,
        }
    );
}

#[path = "render_more_tests.rs"]
mod more;
