//! Encoded WP/WPM coverage for Binary Lighting Output command operations.

use super::*;
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use std::time::Duration;

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn database() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let object = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
        property_value: encode_value(value),
        priority,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property(db, &bytes).map(|_| ())
}

fn wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: Vec<(PropertyIdentifier, Option<u32>, PropertyValue, Option<u8>)>,
) -> Result<(), Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .into_iter()
                .map(
                    |(property_identifier, property_array_index, value, priority)| {
                        BACnetPropertyValue {
                            property_identifier,
                            property_array_index,
                            value: encode_value(value),
                            priority,
                        }
                    },
                )
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property_multiple(db, &bytes).map(|_| ())
}

fn read(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

fn blink_count(db: &ObjectDatabase, oid: ObjectIdentifier) -> u64 {
    db.get(&oid).unwrap().binary_lighting_blink_count_internal()
}

fn advance(db: &mut ObjectDatabase, oid: ObjectIdentifier, elapsed: Duration) -> bool {
    db.get_mut(&oid).unwrap().advance_time_internal(elapsed)
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

fn assert_priority_decode_error(error: Error, priority: u8) {
    match error {
        Error::Decoding { message, .. } => assert!(
            message.contains(&format!("priority {priority} out of range 1-16")),
            "unexpected priority error: {message}"
        ),
        other => panic!("expected existing priority decoding error, got {other:?}"),
    }
}

fn assert_services_invalid_tag(error: Error) {
    match error {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::SERVICES.to_raw() as u32);
            assert_eq!(code, ErrorCode::INVALID_TAG.to_raw() as u32);
        }
        other => panic!("expected SERVICES/INVALID_TAG, got {other:?}"),
    }
}

fn configure_eligible_warn_off(db: &mut ObjectDatabase, oid: ObjectIdentifier, seconds: u64) {
    for (property, value, priority) in [
        (
            PropertyIdentifier::BLINK_WARN_ENABLE,
            PropertyValue::Boolean(true),
            None,
        ),
        (
            PropertyIdentifier::EGRESS_TIME,
            PropertyValue::Unsigned(seconds),
            None,
        ),
        (
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
            Some(8),
        ),
    ] {
        wp(db, oid, property, None, value, priority).unwrap();
    }
}

#[test]
fn present_value_named_commands_and_null_decode_at_valid_and_default_priorities() {
    for (value, expected_slot, expected_pv) in [
        (
            PropertyValue::Enumerated(0),
            PropertyValue::Enumerated(0),
            0,
        ),
        (
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(1),
            1,
        ),
        (PropertyValue::Enumerated(2), PropertyValue::Null, 0),
        (
            PropertyValue::Enumerated(3),
            PropertyValue::Enumerated(0),
            0,
        ),
        (PropertyValue::Enumerated(4), PropertyValue::Null, 0),
        (PropertyValue::Enumerated(5), PropertyValue::Null, 0),
        (PropertyValue::Null, PropertyValue::Null, 0),
    ] {
        for priority in [None, Some(1), Some(16)] {
            let (mut db, oid) = database();
            wp(
                &mut db,
                oid,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                value.clone(),
                priority,
            )
            .unwrap();
            let slot = priority.unwrap_or(16) as u32;
            assert_eq!(
                read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(slot)),
                expected_slot
            );
            assert_eq!(
                read(&db, oid, PropertyIdentifier::PRESENT_VALUE, None),
                PropertyValue::Enumerated(expected_pv)
            );

            let (mut db, oid) = database();
            wpm(
                &mut db,
                oid,
                vec![(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    value.clone(),
                    priority,
                )],
            )
            .unwrap();
            assert_eq!(
                read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(slot)),
                expected_slot
            );
            assert_eq!(
                read(&db, oid, PropertyIdentifier::PRESENT_VALUE, None),
                PropertyValue::Enumerated(expected_pv)
            );
        }
    }
}

#[test]
fn write_property_priority_errors_are_atomic_and_wpm_keeps_prior_prefix() {
    let (mut db, oid) = database();
    configure_eligible_warn_off(&mut db, oid, 5);
    wp(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();

    for priority in [0, 17, u8::MAX] {
        assert_priority_decode_error(
            wp(
                &mut db,
                oid,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(0),
                Some(priority),
            )
            .unwrap_err(),
            priority,
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
            PropertyValue::Boolean(true)
        );
        assert_eq!(blink_count(&db, oid), 1);
    }

    assert_services_invalid_tag(
        wpm(
            &mut db,
            oid,
            vec![
                (
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(1),
                    Some(4),
                ),
                (
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(0),
                    Some(0),
                ),
            ],
        )
        .unwrap_err(),
    );
    assert_eq!(
        read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(4)),
        PropertyValue::Enumerated(1),
        "the valid WPM prefix commits before malformed priority syntax"
    );
    assert_eq!(
        read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
        PropertyValue::Boolean(false)
    );
    assert!(!advance(&mut db, oid, Duration::from_secs(5)));
}

#[test]
fn direct_priority_array_operation_values_are_rejected_with_exact_errors() {
    let (mut db, oid) = database();
    for accepted in [
        PropertyValue::Enumerated(0),
        PropertyValue::Enumerated(1),
        PropertyValue::Null,
    ] {
        wp(
            &mut db,
            oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            accepted,
            None,
        )
        .unwrap();
    }
    for value in [2, 3, 4, 5, 64, 255] {
        assert_property_error(
            wp(
                &mut db,
                oid,
                PropertyIdentifier::PRIORITY_ARRAY,
                Some(8),
                PropertyValue::Enumerated(value),
                None,
            )
            .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)),
            PropertyValue::Null
        );
    }
    assert_property_error(
        wp(
            &mut db,
            oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_property_error(
        wp(
            &mut db,
            oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Null,
            None,
        )
        .unwrap_err(),
        ErrorCode::INVALID_ARRAY_INDEX,
    );

    assert_property_error(
        wpm(
            &mut db,
            oid,
            vec![
                (
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(1),
                    Some(4),
                ),
                (
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(8),
                    PropertyValue::Enumerated(2),
                    None,
                ),
            ],
        )
        .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(4)),
        PropertyValue::Enumerated(1),
        "a later priority-array rejection keeps the earlier write"
    );
}

#[test]
fn eligible_warn_warn_off_warn_relinquish_and_stop_execute_over_wp() {
    let (mut warn_db, warn_oid) = database();
    configure_eligible_warn_off(&mut warn_db, warn_oid, 2);
    wp(
        &mut warn_db,
        warn_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        Some(8),
    )
    .unwrap();
    assert_eq!(blink_count(&warn_db, warn_oid), 1);
    assert_eq!(
        read(
            &warn_db,
            warn_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8)
        ),
        PropertyValue::Enumerated(1)
    );

    let (mut off_db, off_oid) = database();
    configure_eligible_warn_off(&mut off_db, off_oid, 2);
    wp(
        &mut off_db,
        off_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();
    assert!(advance(&mut off_db, off_oid, Duration::from_secs(2)));
    assert_eq!(
        read(
            &off_db,
            off_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8)
        ),
        PropertyValue::Enumerated(0)
    );

    let (mut relinquish_db, relinquish_oid) = database();
    configure_eligible_warn_off(&mut relinquish_db, relinquish_oid, 2);
    wp(
        &mut relinquish_db,
        relinquish_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(4),
        Some(8),
    )
    .unwrap();
    assert!(advance(
        &mut relinquish_db,
        relinquish_oid,
        Duration::from_secs(2)
    ));
    assert_eq!(
        read(
            &relinquish_db,
            relinquish_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8)
        ),
        PropertyValue::Null
    );

    let (mut stop_db, stop_oid) = database();
    configure_eligible_warn_off(&mut stop_db, stop_oid, 2);
    wp(
        &mut stop_db,
        stop_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();
    wp(
        &mut stop_db,
        stop_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(5),
        Some(8),
    )
    .unwrap();
    assert!(!advance(&mut stop_db, stop_oid, Duration::from_secs(10)));
    assert_eq!(
        read(
            &stop_db,
            stop_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8)
        ),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn successful_wpm_can_arm_one_operation() {
    let (mut db, oid) = database();
    wpm(
        &mut db,
        oid,
        vec![
            (
                PropertyIdentifier::BLINK_WARN_ENABLE,
                None,
                PropertyValue::Boolean(true),
                None,
            ),
            (
                PropertyIdentifier::EGRESS_TIME,
                None,
                PropertyValue::Unsigned(3),
                None,
            ),
            (
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(1),
                Some(8),
            ),
            (
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(3),
                Some(8),
            ),
        ],
    )
    .unwrap();
    assert_eq!(blink_count(&db, oid), 1);
    assert_eq!(
        read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
        PropertyValue::Boolean(true)
    );
    assert!(advance(&mut db, oid, Duration::from_secs(3)));
}

#[test]
fn failed_wpm_keeps_new_timer_from_successful_prefix() {
    let (mut db, oid) = database();
    configure_eligible_warn_off(&mut db, oid, 5);
    assert_property_error(
        wpm(
            &mut db,
            oid,
            vec![
                (
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(3),
                    Some(8),
                ),
                (
                    PropertyIdentifier::RELINQUISH_DEFAULT,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                ),
            ],
        )
        .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(blink_count(&db, oid), 1);
    assert_eq!(
        read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
        PropertyValue::Boolean(true)
    );
    assert!(advance(&mut db, oid, Duration::from_secs(5)));
    assert_eq!(
        read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn direct_priority_array_wpm_halts_and_failed_wpm_keeps_prefix() {
    let (mut successful_db, successful_oid) = database();
    configure_eligible_warn_off(&mut successful_db, successful_oid, 5);
    wp(
        &mut successful_db,
        successful_oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();
    wpm(
        &mut successful_db,
        successful_oid,
        vec![(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(4),
            PropertyValue::Enumerated(1),
            None,
        )],
    )
    .unwrap();
    assert_eq!(
        read(
            &successful_db,
            successful_oid,
            PropertyIdentifier::EGRESS_ACTIVE,
            None
        ),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(
            &successful_db,
            successful_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8)
        ),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        read(
            &successful_db,
            successful_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(4)
        ),
        PropertyValue::Enumerated(1)
    );

    let (mut db, oid) = database();
    configure_eligible_warn_off(&mut db, oid, 5);
    wp(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();
    assert!(!advance(&mut db, oid, Duration::from_millis(1_500)));
    assert_property_error(
        wpm(
            &mut db,
            oid,
            vec![
                (
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(4),
                    PropertyValue::Enumerated(1),
                    None,
                ),
                (
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(6),
                    Some(4),
                ),
            ],
        )
        .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(blink_count(&db, oid), 1);
    assert_eq!(
        read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(4)),
        PropertyValue::Enumerated(1),
        "the successful direct priority write stays committed"
    );
    assert_eq!(
        read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
        PropertyValue::Boolean(false)
    );
    assert!(!advance(&mut db, oid, Duration::from_millis(3_500)));
}

#[test]
fn invalid_command_does_not_halt_existing_operation() {
    let (mut db, oid) = database();
    configure_eligible_warn_off(&mut db, oid, 2);
    wp(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(3),
        Some(8),
    )
    .unwrap();
    assert_property_error(
        wp(
            &mut db,
            oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(64),
            Some(4),
        )
        .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(blink_count(&db, oid), 1);
    assert_eq!(
        read(&db, oid, PropertyIdentifier::EGRESS_ACTIVE, None),
        PropertyValue::Boolean(true)
    );
    assert!(advance(&mut db, oid, Duration::from_secs(2)));
}
