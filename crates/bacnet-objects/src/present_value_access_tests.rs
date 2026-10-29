use crate::analog::AnalogValueObject;
use crate::binary::BinaryValueObject;
use crate::command_source::test_origin;
use crate::multistate::MultiStateValueObject;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyWriteCapability;
use crate::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

const LOWER_PRIORITY: u8 = 8;
const HIGHER_PRIORITY: u8 = 4;

/// The rows present only under Commandable access.
const COMMANDABLE_ONLY: [P; 7] = [
    P::PRIORITY_ARRAY,
    P::RELINQUISH_DEFAULT,
    P::CURRENT_COMMAND_PRIORITY,
    P::VALUE_SOURCE,
    P::VALUE_SOURCE_ARRAY,
    P::LAST_COMMAND_TIME,
    P::AUDIT_PRIORITY_FILTER,
];

/// One Value object per family, built with `access`, and two different Present_Values each
/// accepts.
fn values(
    access: PresentValueAccess,
) -> [(Box<dyn BACnetObject>, PropertyValue, PropertyValue); 3] {
    [
        (
            Box::new(
                AnalogValueObject::with_access(
                    1,
                    "AV-1",
                    EngineeringUnits::DEGREES_CELSIUS,
                    access,
                )
                .unwrap(),
            ),
            PropertyValue::Real(21.5),
            PropertyValue::Real(30.0),
        ),
        (
            Box::new(BinaryValueObject::with_access(1, "BV-1", access).unwrap()),
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(0),
        ),
        (
            Box::new(MultiStateValueObject::with_access(1, "MSV-1", 3, access).unwrap()),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ),
    ]
}

/// The same three objects built with `new`, and two different Present_Values each accepts.
fn commandable_values() -> [(Box<dyn BACnetObject>, PropertyValue, PropertyValue); 3] {
    [
        (
            Box::new(AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
            PropertyValue::Real(21.5),
            PropertyValue::Real(30.0),
        ),
        (
            Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(0),
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ),
    ]
}

fn assert_protocol<T: std::fmt::Debug>(
    result: Result<T, Error>,
    class: ErrorClass,
    code: ErrorCode,
) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: actual_class,
            code: actual_code,
        })) => {
            assert_eq!(actual_class, class);
            assert_eq!(actual_code, code);
        }
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

/// A peer's WriteProperty to Present_Value at `priority`.
fn write_present_value(
    object: &mut dyn BACnetObject,
    value: PropertyValue,
    priority: u8,
) -> Result<(), Error> {
    object.write_property_from(
        P::PRESENT_VALUE,
        None,
        value,
        Some(priority),
        &test_origin(),
    )
}

fn take_out_of_service(object: &mut dyn BACnetObject) {
    object
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
}

fn present_value(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::PRESENT_VALUE, None).unwrap()
}

fn present_value_capability(object: &dyn BACnetObject) -> PropertyWriteCapability {
    object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == P::PRESENT_VALUE)
        .expect("Present_Value is always present")
        .write_capability
}

/// The highest occupied priority is Present_Value, and with every priority
/// relinquished it is Relinquish_Default.
#[test]
fn new_is_commandable() {
    for (mut object, lower, higher) in commandable_values() {
        let relinquish_default = object.read_property(P::RELINQUISH_DEFAULT, None).unwrap();

        write_present_value(object.as_mut(), lower.clone(), LOWER_PRIORITY).unwrap();
        assert_eq!(present_value(object.as_ref()), lower);

        write_present_value(object.as_mut(), higher.clone(), HIGHER_PRIORITY).unwrap();
        assert_eq!(present_value(object.as_ref()), higher);

        write_present_value(object.as_mut(), lower.clone(), LOWER_PRIORITY).unwrap();
        assert_eq!(
            present_value(object.as_ref()),
            higher,
            "a lower priority must not override a higher one"
        );

        write_present_value(object.as_mut(), PropertyValue::Null, HIGHER_PRIORITY).unwrap();
        assert_eq!(present_value(object.as_ref()), lower);

        write_present_value(object.as_mut(), PropertyValue::Null, LOWER_PRIORITY).unwrap();
        assert_eq!(present_value(object.as_ref()), relinquish_default);
    }
}

#[test]
fn read_only_and_writable_have_no_command_prioritization() {
    for access in [PresentValueAccess::ReadOnly, PresentValueAccess::Writable] {
        for (mut object, _, _) in values(access) {
            for property in COMMANDABLE_ONLY {
                assert!(
                    !object.property_list().contains(&property),
                    "{access:?} lists {property:?}"
                );
                assert_protocol(
                    object.read_property(property, None),
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }

            assert_protocol(
                object.write_property(P::RELINQUISH_DEFAULT, None, PropertyValue::Null, None),
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_protocol(
                object.write_property_from(
                    P::VALUE_SOURCE,
                    None,
                    PropertyValue::Null,
                    Some(LOWER_PRIORITY),
                    &test_origin(),
                ),
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
    }
}

#[test]
fn read_only_refuses_peer_writes_while_in_service() {
    for (mut object, value, _) in values(PresentValueAccess::ReadOnly) {
        let before = present_value(object.as_ref());

        assert_protocol(
            write_present_value(object.as_mut(), value, LOWER_PRIORITY),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(present_value(object.as_ref()), before);
        assert_eq!(
            present_value_capability(object.as_ref()),
            PropertyWriteCapability::WhenOutOfService
        );
    }
}

/// Out_Of_Service TRUE makes Present_Value writable for testing.
#[test]
fn read_only_accepts_peer_writes_while_out_of_service() {
    for (mut object, value, _) in values(PresentValueAccess::ReadOnly) {
        take_out_of_service(object.as_mut());

        write_present_value(object.as_mut(), value.clone(), LOWER_PRIORITY).unwrap();

        assert_eq!(present_value(object.as_ref()), value);
    }
}

/// Each write replaces Present_Value, whatever priority it carries.
#[test]
fn writable_takes_every_peer_write_directly() {
    for (mut object, first, second) in values(PresentValueAccess::Writable) {
        write_present_value(object.as_mut(), first, HIGHER_PRIORITY).unwrap();
        write_present_value(object.as_mut(), second.clone(), LOWER_PRIORITY).unwrap();

        assert_eq!(present_value(object.as_ref()), second);
        assert_eq!(
            present_value_capability(object.as_ref()),
            PropertyWriteCapability::Always
        );
    }
}

#[test]
fn the_application_sets_present_value_while_in_service() {
    for access in [PresentValueAccess::ReadOnly, PresentValueAccess::Writable] {
        for (mut object, value, _) in values(access) {
            object.set_present_value_internal(value.clone()).unwrap();

            assert_eq!(present_value(object.as_ref()), value, "{access:?}");
        }
    }
}

/// Out_Of_Service TRUE keeps software local to the device from changing Present_Value.
#[test]
fn the_application_is_refused_while_out_of_service() {
    for access in [PresentValueAccess::ReadOnly, PresentValueAccess::Writable] {
        for (mut object, value, _) in values(access) {
            take_out_of_service(object.as_mut());
            let before = present_value(object.as_ref());

            assert_protocol(
                object.set_present_value_internal(value),
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(present_value(object.as_ref()), before, "{access:?}");
        }
    }
}

#[test]
fn the_application_does_not_set_a_commandable_present_value() {
    for (mut object, value, _) in commandable_values() {
        assert_protocol(
            object.set_present_value_internal(value),
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
    }
}

#[test]
fn a_value_of_the_wrong_kind_is_refused() {
    for access in [PresentValueAccess::ReadOnly, PresentValueAccess::Writable] {
        for (mut object, _, _) in values(access) {
            let before = present_value(object.as_ref());

            assert_protocol(
                object.set_present_value_internal(PropertyValue::Boolean(true)),
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(present_value(object.as_ref()), before, "{access:?}");
        }
    }
}

fn property_write(
    object: &mut dyn BACnetObject,
    sourced: bool,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    if sourced {
        object.write_property_from(P::PRESENT_VALUE, index, value, priority, &test_origin())
    } else {
        object.write_property(P::PRESENT_VALUE, index, value, priority)
    }
}

#[test]
fn noncommandable_network_null_is_a_successful_noop() {
    for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
        for sourced in [true, false] {
            for (mut object, value, _) in values(access) {
                object.set_present_value_internal(value.clone()).unwrap();
                if access == PresentValueAccess::ReadOnly {
                    take_out_of_service(object.as_mut());
                }
                for priority in [None, Some(1), Some(6), Some(16)] {
                    property_write(
                        object.as_mut(),
                        sourced,
                        None,
                        PropertyValue::Null,
                        priority,
                    )
                    .unwrap();
                    assert_eq!(present_value(object.as_ref()), value);
                }
            }
        }
    }
}

#[test]
fn noncommandable_context_free_writes_match_sourced_writes() {
    for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
        for sourced in [false, true] {
            for (mut object, first, second) in values(access) {
                if access == PresentValueAccess::ReadOnly {
                    let before = present_value(object.as_ref());
                    for value in [first.clone(), PropertyValue::Null] {
                        assert_protocol(
                            property_write(object.as_mut(), sourced, None, value, None),
                            ErrorClass::PROPERTY,
                            ErrorCode::WRITE_ACCESS_DENIED,
                        );
                        assert_eq!(present_value(object.as_ref()), before);
                    }
                    take_out_of_service(object.as_mut());
                }
                for (value, priority) in [(first, None), (second, Some(6))] {
                    property_write(object.as_mut(), sourced, None, value.clone(), priority)
                        .unwrap();
                    assert_eq!(present_value(object.as_ref()), value);
                }
            }
        }
    }
}

#[test]
fn noncommandable_write_errors_preserve_value_and_internal_null_is_invalid() {
    for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
        for sourced in [false, true] {
            for ((mut object, value, _), invalid_range) in values(access).into_iter().zip([
                PropertyValue::Real(f32::NAN),
                PropertyValue::Enumerated(2),
                PropertyValue::Unsigned(4),
            ]) {
                object.set_present_value_internal(value.clone()).unwrap();
                assert_protocol(
                    object.set_present_value_internal(PropertyValue::Null),
                    ErrorClass::PROPERTY,
                    ErrorCode::INVALID_DATA_TYPE,
                );
                assert_eq!(present_value(object.as_ref()), value);
                if access == PresentValueAccess::ReadOnly {
                    take_out_of_service(object.as_mut());
                }
                for (index, invalid, code) in [
                    (
                        None,
                        PropertyValue::Boolean(true),
                        ErrorCode::INVALID_DATA_TYPE,
                    ),
                    (None, invalid_range, ErrorCode::VALUE_OUT_OF_RANGE),
                    (
                        Some(0),
                        PropertyValue::Null,
                        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                    ),
                    (Some(1), value.clone(), ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                ] {
                    assert_protocol(
                        property_write(object.as_mut(), sourced, index, invalid, Some(6)),
                        ErrorClass::PROPERTY,
                        code,
                    );
                    assert_eq!(present_value(object.as_ref()), value);
                }
            }
        }
    }
}

#[test]
fn commandable_context_free_present_value_still_requires_origin() {
    for (mut object, value, _) in commandable_values() {
        let before = present_value(object.as_ref());
        for value in [value, PropertyValue::Null] {
            assert_protocol(
                object.write_property(P::PRESENT_VALUE, None, value, Some(8)),
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(present_value(object.as_ref()), before);
        }
    }
}
