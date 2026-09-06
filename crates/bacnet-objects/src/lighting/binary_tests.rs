use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

fn write(object: &mut BinaryLightingOutputObject, value: PropertyValue, priority: u8) {
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            Some(priority),
        )
        .unwrap();
}

fn set_blink(object: &mut BinaryLightingOutputObject, enabled: bool) {
    object
        .write_property(
            PropertyIdentifier::BLINK_WARN_ENABLE,
            None,
            PropertyValue::Boolean(enabled),
            None,
        )
        .unwrap();
}

fn set_egress(object: &mut BinaryLightingOutputObject, seconds: u64) {
    object
        .write_property(
            PropertyIdentifier::EGRESS_TIME,
            None,
            PropertyValue::Unsigned(seconds),
            None,
        )
        .unwrap();
}

fn read(object: &BinaryLightingOutputObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn slot(object: &BinaryLightingOutputObject, priority: u32) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
        .unwrap()
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

fn armed_warn_off(seconds: u64) -> BinaryLightingOutputObject {
    let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
    set_blink(&mut object, true);
    set_egress(&mut object, seconds);
    write(&mut object, PropertyValue::Enumerated(ON), 8);
    write(&mut object, PropertyValue::Enumerated(3), 8);
    object
}

#[test]
fn startup_and_all_reads_contain_only_steady_values() {
    let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(object.binary_lighting_blink_count_internal(), 0);

    set_blink(&mut object, true);
    for operation in 2..=5 {
        write(&mut object, PropertyValue::Enumerated(ON), 8);
        write(&mut object, PropertyValue::Enumerated(operation), 8);
        if operation == 3 || operation == 4 {
            object.advance_time_internal(Duration::from_secs(u32::MAX as u64));
        }
        assert!(matches!(
            read(&object, PropertyIdentifier::PRESENT_VALUE),
            PropertyValue::Enumerated(OFF | ON)
        ));
        let PropertyValue::List(priority_array) = read(&object, PropertyIdentifier::PRIORITY_ARRAY)
        else {
            panic!("priority array must read as a list");
        };
        assert!(priority_array.into_iter().all(|value| matches!(
            value,
            PropertyValue::Null | PropertyValue::Enumerated(OFF | ON)
        )));
    }
}

#[test]
fn warn_requests_blink_only_for_existing_highest_on_slot() {
    let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
    set_blink(&mut object, true);

    write(&mut object, PropertyValue::Enumerated(2), 8);
    assert_eq!(
        read(&object, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Enumerated(OFF)
    );
    assert_eq!(
        slot(&object, 8),
        PropertyValue::Null,
        "WARN must not synthesize ON"
    );

    write(&mut object, PropertyValue::Enumerated(ON), 8);
    write(&mut object, PropertyValue::Enumerated(2), 8);
    assert_eq!(object.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );

    for priority in [7, 9] {
        write(&mut object, PropertyValue::Enumerated(2), priority);
    }
    set_blink(&mut object, false);
    write(&mut object, PropertyValue::Enumerated(2), 8);
    assert_eq!(object.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
}

#[test]
fn warn_off_arms_snapshotted_delay_and_expires_exactly_once() {
    let mut object = armed_warn_off(5);
    assert_eq!(object.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
    assert_eq!(
        read(&object, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Enumerated(ON)
    );
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(true)
    );

    assert!(!object.advance_time_internal(Duration::from_millis(4_999)));
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
    assert!(object.advance_time_internal(Duration::from_millis(1)));
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(OFF));
    assert_eq!(
        read(&object, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Enumerated(OFF)
    );
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert!(!object.advance_time_internal(Duration::from_secs(100)));
}

#[test]
fn warn_off_immediate_paths_and_zero_duration_do_not_leave_active_egress() {
    for (blink, prewrite, command_priority) in [
        (false, None, 8),
        (true, Some((8, OFF)), 8),
        (true, Some((4, ON)), 8),
    ] {
        let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
        set_blink(&mut object, blink);
        set_egress(&mut object, 5);
        if let Some((priority, value)) = prewrite {
            write(&mut object, PropertyValue::Enumerated(value), priority);
        }
        write(&mut object, PropertyValue::Enumerated(3), command_priority);
        assert_eq!(
            slot(&object, command_priority as u32),
            PropertyValue::Enumerated(OFF)
        );
        assert_eq!(
            read(&object, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(false)
        );
        assert_eq!(object.binary_lighting_blink_count_internal(), 0);
    }

    let mut zero = BinaryLightingOutputObject::new(1, "zero").unwrap();
    set_blink(&mut zero, true);
    write(&mut zero, PropertyValue::Enumerated(ON), 8);
    write(&mut zero, PropertyValue::Enumerated(3), 8);
    assert_eq!(zero.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&zero, 8), PropertyValue::Enumerated(OFF));
    assert_eq!(
        read(&zero, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn warn_relinquish_arms_only_when_the_next_effective_value_is_not_on() {
    let mut eligible = BinaryLightingOutputObject::new(1, "eligible").unwrap();
    set_blink(&mut eligible, true);
    set_egress(&mut eligible, 3);
    write(&mut eligible, PropertyValue::Enumerated(ON), 8);
    write(&mut eligible, PropertyValue::Enumerated(4), 8);
    assert_eq!(eligible.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&eligible, 8), PropertyValue::Enumerated(ON));
    assert!(!eligible.advance_time_internal(Duration::from_millis(2_999)));
    assert!(eligible.advance_time_internal(Duration::from_millis(1)));
    assert_eq!(slot(&eligible, 8), PropertyValue::Null);
    assert_eq!(
        read(&eligible, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Enumerated(OFF)
    );

    let mut next_on = BinaryLightingOutputObject::new(2, "next-on").unwrap();
    set_blink(&mut next_on, true);
    set_egress(&mut next_on, 3);
    write(&mut next_on, PropertyValue::Enumerated(ON), 10);
    write(&mut next_on, PropertyValue::Enumerated(ON), 8);
    write(&mut next_on, PropertyValue::Enumerated(4), 8);
    assert_eq!(next_on.binary_lighting_blink_count_internal(), 0);
    assert_eq!(slot(&next_on, 8), PropertyValue::Null);
    assert_eq!(
        read(&next_on, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Enumerated(ON)
    );
    assert_eq!(
        read(&next_on, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );

    for (blink, initial) in [(false, ON), (true, OFF)] {
        let mut immediate = BinaryLightingOutputObject::new(3, "immediate").unwrap();
        set_blink(&mut immediate, blink);
        set_egress(&mut immediate, 3);
        write(&mut immediate, PropertyValue::Enumerated(initial), 8);
        write(&mut immediate, PropertyValue::Enumerated(4), 8);
        assert_eq!(slot(&immediate, 8), PropertyValue::Null);
        assert_eq!(
            read(&immediate, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(false)
        );
    }
}

#[test]
fn warn_relinquish_not_highest_and_zero_duration_take_immediate_paths() {
    let mut not_highest = BinaryLightingOutputObject::new(1, "not-highest").unwrap();
    set_blink(&mut not_highest, true);
    set_egress(&mut not_highest, 5);
    write(&mut not_highest, PropertyValue::Enumerated(ON), 8);
    write(&mut not_highest, PropertyValue::Enumerated(ON), 4);
    write(&mut not_highest, PropertyValue::Enumerated(4), 8);
    assert_eq!(slot(&not_highest, 8), PropertyValue::Null);
    assert_eq!(not_highest.binary_lighting_blink_count_internal(), 0);
    assert_eq!(
        read(&not_highest, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );

    let mut zero = BinaryLightingOutputObject::new(2, "zero").unwrap();
    set_blink(&mut zero, true);
    write(&mut zero, PropertyValue::Enumerated(ON), 8);
    write(&mut zero, PropertyValue::Enumerated(4), 8);
    assert_eq!(zero.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&zero, 8), PropertyValue::Null);
    assert_eq!(
        read(&zero, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn stop_is_same_priority_only_and_idempotent() {
    let mut object = armed_warn_off(5);
    write(&mut object, PropertyValue::Enumerated(5), 7);
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(true)
    );

    write(&mut object, PropertyValue::Enumerated(5), 8);
    write(&mut object, PropertyValue::Enumerated(5), 8);
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
    assert!(!object.advance_time_internal(Duration::from_secs(100)));
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
}

#[test]
fn same_and_higher_commands_complete_old_operation_before_incoming_write() {
    let mut same = armed_warn_off(5);
    write(&mut same, PropertyValue::Enumerated(ON), 8);
    assert_eq!(
        slot(&same, 8),
        PropertyValue::Enumerated(ON),
        "incoming ordinary command wins"
    );
    assert_eq!(
        read(&same, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );

    let mut higher = armed_warn_off(5);
    write(&mut higher, PropertyValue::Enumerated(ON), 4);
    assert_eq!(slot(&higher, 8), PropertyValue::Enumerated(OFF));
    assert_eq!(slot(&higher, 4), PropertyValue::Enumerated(ON));
    assert_eq!(
        read(&higher, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );

    let mut relinquish = BinaryLightingOutputObject::new(3, "relinquish").unwrap();
    set_blink(&mut relinquish, true);
    set_egress(&mut relinquish, 5);
    write(&mut relinquish, PropertyValue::Enumerated(ON), 8);
    write(&mut relinquish, PropertyValue::Enumerated(4), 8);
    write(&mut relinquish, PropertyValue::Enumerated(ON), 8);
    assert_eq!(slot(&relinquish, 8), PropertyValue::Enumerated(ON));
    assert_eq!(
        read(&relinquish, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn ordinary_same_priority_off_on_and_null_win_after_each_operation_kind() {
    for operation in [3, 4] {
        for (incoming, expected) in [
            (
                PropertyValue::Enumerated(OFF),
                PropertyValue::Enumerated(OFF),
            ),
            (PropertyValue::Enumerated(ON), PropertyValue::Enumerated(ON)),
            (PropertyValue::Null, PropertyValue::Null),
        ] {
            let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
            set_blink(&mut object, true);
            set_egress(&mut object, 5);
            write(&mut object, PropertyValue::Enumerated(ON), 8);
            write(&mut object, PropertyValue::Enumerated(operation), 8);
            write(&mut object, incoming, 8);
            assert_eq!(slot(&object, 8), expected);
            assert_eq!(
                read(&object, PropertyIdentifier::EGRESS_ACTIVE),
                PropertyValue::Boolean(false)
            );
        }
    }
}

#[test]
fn lower_and_repeated_special_commands_cannot_create_a_second_timer() {
    let mut lower = armed_warn_off(5);
    write(&mut lower, PropertyValue::Enumerated(3), 10);
    assert_eq!(slot(&lower, 10), PropertyValue::Enumerated(OFF));
    assert_eq!(lower.active_operation.unwrap().priority, 8);
    assert_eq!(lower.binary_lighting_blink_count_internal(), 1);

    let mut repeated = armed_warn_off(5);
    write(&mut repeated, PropertyValue::Enumerated(3), 8);
    assert_eq!(slot(&repeated, 8), PropertyValue::Enumerated(OFF));
    assert_eq!(
        read(&repeated, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(repeated.binary_lighting_blink_count_internal(), 1);

    let mut repeated_relinquish = BinaryLightingOutputObject::new(2, "repeat-wr").unwrap();
    set_blink(&mut repeated_relinquish, true);
    set_egress(&mut repeated_relinquish, 5);
    write(&mut repeated_relinquish, PropertyValue::Enumerated(ON), 8);
    write(&mut repeated_relinquish, PropertyValue::Enumerated(4), 8);
    write(&mut repeated_relinquish, PropertyValue::Enumerated(4), 8);
    assert_eq!(slot(&repeated_relinquish, 8), PropertyValue::Null);
    assert_eq!(
        read(&repeated_relinquish, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        repeated_relinquish.binary_lighting_blink_count_internal(),
        1
    );
}

#[test]
fn invalid_commands_are_side_effect_free_before_halt() {
    let mut object = armed_warn_off(5);
    let before_count = object.binary_lighting_blink_count_internal();
    for (value, priority, expected) in [
        (
            PropertyValue::Enumerated(6),
            Some(4),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Enumerated(OFF),
            Some(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Unsigned(1),
            Some(4),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let error = object
            .write_property(PropertyIdentifier::PRESENT_VALUE, None, value, priority)
            .unwrap_err();
        assert_property_error(error, expected);
        assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
        assert_eq!(
            read(&object, PropertyIdentifier::EGRESS_ACTIVE),
            PropertyValue::Boolean(true)
        );
        assert_eq!(object.binary_lighting_blink_count_internal(), before_count);
    }
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
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(true)
    );
    assert!(!object.advance_time_internal(Duration::from_millis(4_999)));
    assert!(object.advance_time_internal(Duration::from_millis(1)));
}

#[test]
fn direct_priority_array_accepts_only_off_on_and_null_with_exact_errors() {
    let mut object = BinaryLightingOutputObject::new(1, "BLO").unwrap();
    for value in [
        PropertyValue::Enumerated(OFF),
        PropertyValue::Enumerated(ON),
        PropertyValue::Null,
    ] {
        object
            .write_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8), value, None)
            .unwrap();
    }
    for value in [2, 3, 4, 5, 63, 64, 255, u32::MAX] {
        assert_property_error(
            object
                .write_property(
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(8),
                    PropertyValue::Enumerated(value),
                    None,
                )
                .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(slot(&object, 8), PropertyValue::Null);
    }
    assert_property_error(
        object
            .write_property(
                PropertyIdentifier::PRIORITY_ARRAY,
                Some(8),
                PropertyValue::Unsigned(ON as u64),
                None,
            )
            .unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
    for index in [Some(0), Some(17)] {
        assert_property_error(
            object
                .write_property(
                    PropertyIdentifier::PRIORITY_ARRAY,
                    index,
                    PropertyValue::Null,
                    None,
                )
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
    assert_property_error(
        object
            .write_property(
                PropertyIdentifier::PRIORITY_ARRAY,
                None,
                PropertyValue::Null,
                None,
            )
            .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn out_of_service_and_post_arm_configuration_changes_do_not_affect_active_timer() {
    let mut object = armed_warn_off(5);
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    set_egress(&mut object, 100);
    set_blink(&mut object, false);
    assert!(!object.advance_time_internal(Duration::from_millis(4_999)));
    assert!(object.advance_time_internal(Duration::from_millis(1)));
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(OFF));

    write(&mut object, PropertyValue::Enumerated(ON), 8);
    write(&mut object, PropertyValue::Enumerated(3), 8);
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(OFF));
}

#[test]
fn fractional_and_large_elapsed_values_are_safe() {
    let mut fractional = armed_warn_off(2);
    assert!(!fractional.advance_time_internal(Duration::from_millis(1_500)));
    assert!(!fractional.advance_time_internal(Duration::from_millis(499)));
    assert!(fractional.advance_time_internal(Duration::from_millis(1)));

    let mut large = armed_warn_off(u32::MAX as u64);
    assert!(large.advance_time_internal(Duration::from_secs(u64::MAX)));
    assert_eq!(slot(&large, 8), PropertyValue::Enumerated(OFF));
}

#[test]
fn rollback_restores_operation_remaining_time_and_blink_observation_exactly() {
    let mut object = armed_warn_off(5);
    assert!(!object.advance_time_internal(Duration::from_millis(1_500)));
    let rollback = object
        .capture_write_property_rollback(
            PropertyIdentifier::PRESENT_VALUE,
            &PropertyValue::Enumerated(ON),
        )
        .unwrap();
    write(&mut object, PropertyValue::Enumerated(ON), 4);
    object.restore_write_property_rollback(rollback).unwrap();

    assert_eq!(object.binary_lighting_blink_count_internal(), 1);
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(ON));
    assert_eq!(
        read(&object, PropertyIdentifier::EGRESS_ACTIVE),
        PropertyValue::Boolean(true)
    );
    assert!(!object.advance_time_internal(Duration::from_millis(3_499)));
    assert!(object.advance_time_internal(Duration::from_millis(1)));
    assert_eq!(slot(&object, 8), PropertyValue::Enumerated(OFF));
}
