use std::time::Duration;

use bacnet_types::{
    enums::{ErrorClass, ErrorCode, PropertyIdentifier},
    error::Error,
    primitives::PropertyValue,
};

use super::{BinaryLightingOutputObject, OFF, ON};
use crate::traits::BACnetObject;

fn read(object: &BinaryLightingOutputObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn slot(object: &BinaryLightingOutputObject, priority: u8) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority as u32))
        .unwrap()
}

fn write_present(object: &mut BinaryLightingOutputObject, value: u32, priority: u8) {
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(value),
            Some(priority),
        )
        .unwrap();
}

fn write_direct(object: &mut BinaryLightingOutputObject, priority: u8, value: PropertyValue) {
    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(priority as u32),
            value,
            None,
        )
        .unwrap();
}

fn armed(operation: u32) -> BinaryLightingOutputObject {
    let mut object = BinaryLightingOutputObject::new(1, "direct ordering").unwrap();
    object
        .write_property(
            PropertyIdentifier::BLINK_WARN_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::EGRESS_TIME,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
    write_present(&mut object, ON, 8);
    write_present(&mut object, operation, 8);
    object
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    match error {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, expected.to_raw() as u32);
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

#[test]
fn same_priority_direct_values_win_after_both_operation_kinds() {
    for operation in [3, 4] {
        for incoming in [
            PropertyValue::Enumerated(OFF),
            PropertyValue::Enumerated(ON),
            PropertyValue::Null,
        ] {
            let mut object = armed(operation);
            write_direct(&mut object, 8, incoming.clone());

            assert_eq!(slot(&object, 8), incoming);
            assert_eq!(
                read(&object, PropertyIdentifier::EGRESS_ACTIVE),
                PropertyValue::Boolean(false)
            );
            assert!(!object.advance_time_internal(Duration::from_secs(10)));
        }
    }
}

#[test]
fn higher_priority_direct_write_completes_old_slot_then_installs_incoming() {
    for (operation, completed_slot) in [
        (3, PropertyValue::Enumerated(OFF)),
        (4, PropertyValue::Null),
    ] {
        let mut object = armed(operation);
        write_direct(&mut object, 4, PropertyValue::Enumerated(ON));

        assert_eq!(slot(&object, 8), completed_slot);
        assert_eq!(slot(&object, 4), PropertyValue::Enumerated(ON));
        assert_eq!(
            read(&object, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(false)
        );
    }
}

#[test]
fn lower_priority_direct_write_preserves_each_operation_and_remaining_time() {
    for (operation, completed_slot) in [
        (3, PropertyValue::Enumerated(OFF)),
        (4, PropertyValue::Null),
    ] {
        let mut object = armed(operation);
        assert!(!object.advance_time_internal(Duration::from_millis(1_500)));
        write_direct(&mut object, 10, PropertyValue::Enumerated(ON));

        assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
        assert_eq!(slot(&object, 10), PropertyValue::Enumerated(ON));
        assert_eq!(
            read(&object, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(true)
        );
        assert!(!object.advance_time_internal(Duration::from_millis(3_499)));
        assert!(object.advance_time_internal(Duration::from_millis(1)));
        assert_eq!(slot(&object, 8), completed_slot);
    }
}

#[test]
fn invalid_direct_writes_preserve_each_operation_and_remaining_time() {
    for operation in [3, 4] {
        let mut object = armed(operation);
        assert!(!object.advance_time_internal(Duration::from_millis(1_500)));
        let blink_count = object.binary_lighting_blink_count_internal();

        assert_property_error(
            object
                .write_property(
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(4),
                    PropertyValue::Enumerated(3),
                    None,
                )
                .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_property_error(
            object
                .write_property(
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(4),
                    PropertyValue::Unsigned(ON as u64),
                    None,
                )
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_property_error(
            object
                .write_property(
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(17),
                    PropertyValue::Null,
                    None,
                )
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );

        assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
        assert_eq!(slot(&object, 4), PropertyValue::Null);
        assert_eq!(object.binary_lighting_blink_count_internal(), blink_count);
        assert_eq!(
            read(&object, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(true)
        );
        assert!(!object.advance_time_internal(Duration::from_millis(3_499)));
        assert!(object.advance_time_internal(Duration::from_millis(1)));
    }
}
