use super::super::*;
use crate::event::test_clock::{test_now, test_tick};
use crate::property_metadata::is_writable_property;
use crate::traits::EventReporting as _;

#[test]
fn bv_read_present_value_default() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // inactive
}

#[test]
fn bv_write_present_value() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // active
        Some(8),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
}

#[test]
fn bv_write_invalid_value_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let result = bv.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2), // invalid -- only 0 or 1
        Some(8),
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bv_write_wrong_type_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let result = bv.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0), // wrong type
        Some(8),
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bv_read_object_type() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw())
    );
}

#[test]
fn bv_read_reliability_default() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- BinaryInput ---

#[test]
fn bi_read_present_value_default() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let val = bi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn bi_write_denied_when_in_service() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let result = bi.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        None,
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bi_write_allowed_when_out_of_service() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    bi.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        None,
        &crate::command_source::test_origin(),
    )
    .unwrap();
    let val = bi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
}

#[test]
fn bi_set_present_value() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(1);
    let val = bi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
}

#[test]
fn bi_read_polarity_default() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let val = bi
        .read_property(PropertyIdentifier::POLARITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn bi_read_reliability_default() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let val = bi
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- BinaryOutput ---

#[test]
fn bo_write_with_priority() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    let val = bo
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
    let slot = bo
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Enumerated(1));
}

#[test]
fn bo_relinquish_falls_to_default() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(16),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn bo_invalid_value_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let result = bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        None,
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bo_read_polarity_default() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::POLARITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn bo_read_reliability_default() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

#[test]
fn bi_detection_enable_resets_and_gates_intrinsic_reporting() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    bi.event_detector.alarm_values = vec![1];
    bi.event_detector.time_delay = 2;
    bi.set_present_value(1);
    assert_eq!(bi.evaluate_intrinsic_reporting(test_now()), None);
    assert!(bi.event_detector.pending.is_some());

    bi.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
    bi.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
    bi.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
    bi.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    assert_eq!(
        bi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        bi.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw())
    );
    assert_eq!(
        bi.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe0],
        }
    );
    assert!(bi.event_detector.pending.is_none());
    assert!(bi.event_detector.fault_reliability.is_none());
    assert_eq!(bi.evaluate_intrinsic_reporting(test_now()), None);
    assert_eq!(bi.tick_intrinsic_reporting(test_tick()), None);
    assert!(
        bi.event_detector.pending.is_none(),
        "evaluate/tick re-armed a countdown while detection is disabled"
    );
    assert!(crate::property_metadata::has_property(
        &bi.property_metadata(),
        PropertyIdentifier::EVENT_DETECTION_ENABLE
    ));
    assert!(is_writable_property(
        &bi,
        PropertyIdentifier::EVENT_DETECTION_ENABLE
    ));
}

// --- Priority array bounds tests (BinaryOutput) ---

#[test]
fn bo_priority_array_index_zero_returns_size() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(16));
}

#[test]
fn bo_priority_array_index_out_of_bounds() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Index 17 is out of bounds (valid: 0-16)
    let result = bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
    assert!(result.is_err());
}

#[test]
fn bo_priority_array_index_far_out_of_bounds() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let result = bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(100));
    assert!(result.is_err());
}

// --- WriteProperty with invalid priority tests (BinaryOutput) ---

#[test]
fn bo_write_with_priority_zero_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Priority 0 is invalid (valid range is 1-16)
    let result = bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(0),
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bo_write_with_priority_17_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Priority 17 is invalid (valid range is 1-16)
    let result = bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(17),
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

#[test]
fn bo_write_with_priority_255_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Priority 255 is invalid
    let result = bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(255),
        &crate::command_source::test_origin(),
    );
    assert!(result.is_err());
}

// --- BinaryInput read-only properties ---

#[test]
fn bi_polarity_is_readable_as_enumerated() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let val = bi
        .read_property(PropertyIdentifier::POLARITY, None)
        .unwrap();
    // Polarity default is 0 (normal), verify it comes back as Enumerated
    match val {
        PropertyValue::Enumerated(v) => assert_eq!(v, 0),
        other => panic!("Expected Enumerated for POLARITY, got {:?}", other),
    }
}

#[test]
fn bi_reliability_is_readable_as_enumerated() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let val = bi
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    // Reliability default is 0 (NO_FAULT_DETECTED), verify correct type
    match val {
        PropertyValue::Enumerated(v) => assert_eq!(v, 0),
        other => panic!("Expected Enumerated for RELIABILITY, got {:?}", other),
    }
}

#[test]
fn bo_polarity_is_readable_as_enumerated() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::POLARITY, None)
        .unwrap();
    match val {
        PropertyValue::Enumerated(v) => assert_eq!(v, 0),
        other => panic!("Expected Enumerated for POLARITY, got {:?}", other),
    }
}

#[test]
fn bo_reliability_is_readable_as_enumerated() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    match val {
        PropertyValue::Enumerated(v) => assert_eq!(v, 0),
        other => panic!("Expected Enumerated for RELIABILITY, got {:?}", other),
    }
}

#[test]
fn bi_polarity_in_property_list() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let props = crate::test_rows::listed(&bi.property_metadata());
    assert!(props.contains(&PropertyIdentifier::POLARITY));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
}

#[test]
fn bo_priority_array_read_all_slots_none_by_default() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let val = bo
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    if let PropertyValue::List(elements) = val {
        assert_eq!(elements.len(), 16);
        for elem in &elements {
            assert_eq!(elem, &PropertyValue::Null);
        }
    } else {
        panic!("Expected List for priority array without index");
    }
}

// --- Present_Value commands and read-only Priority_Array writes ---

#[test]
fn bo_present_value_priority_write_value() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // active
        Some(5),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn bo_present_value_priority_relinquish() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(5),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(5),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    // Fall back to relinquish default (0 = inactive)
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn bo_direct_priority_array_no_index_error() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let result = bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        PropertyValue::Enumerated(1),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn bo_direct_priority_array_index_zero_error() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let result = bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        PropertyValue::Enumerated(1),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn bo_direct_priority_array_index_17_error() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let result = bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(17),
        PropertyValue::Enumerated(1),
        None,
    );
    assert!(result.is_err());
}

// ── Trait capability method tests (issue #115 shared truth source) ──────────

#[test]
fn bi_is_writable_property_mirrors_write_property() {
    let bi = BinaryInputObject::new(1, "bi-1").unwrap();
    // BI accepts PRESENT_VALUE (when OOS), ACTIVE/INACTIVE_TEXT, common set.
    assert!(is_writable_property(&bi, PropertyIdentifier::PRESENT_VALUE));
    assert!(is_writable_property(&bi, PropertyIdentifier::ACTIVE_TEXT));
    assert!(is_writable_property(&bi, PropertyIdentifier::INACTIVE_TEXT));
    assert!(is_writable_property(
        &bi,
        PropertyIdentifier::OUT_OF_SERVICE
    ));
    assert!(is_writable_property(&bi, PropertyIdentifier::OBJECT_NAME));
    assert!(is_writable_property(&bi, PropertyIdentifier::DESCRIPTION));
    // Writable since #229: Clause 12.6 requires the supported Event_Enable
    // value set to include (T, T, T); writability is this stack's route to
    // that minimum, since the detectors default to (F, F, F) and had no
    // other commissioning path.
    assert!(is_writable_property(&bi, PropertyIdentifier::EVENT_ENABLE));
    assert!(is_writable_property(
        &bi,
        PropertyIdentifier::NOTIFICATION_CLASS
    ));
    assert!(is_writable_property(&bi, PropertyIdentifier::NOTIFY_TYPE));
    assert!(is_writable_property(&bi, PropertyIdentifier::TIME_DELAY));
    // ACKED_TRANSITIONS stays read-only: the alarm-acknowledgment process
    // maintains it from event-state transitions and acknowledgment
    // indications, never a property write.
    assert!(!is_writable_property(
        &bi,
        PropertyIdentifier::ACKED_TRANSITIONS
    ));
    // Not commandable.
    assert!(!is_writable_property(
        &bi,
        PropertyIdentifier::PRIORITY_ARRAY
    ));
}

#[test]
fn bo_is_writable_property_mirrors_write_property() {
    let bo = BinaryOutputObject::new(1, "bo-1").unwrap();
    // Commandable.
    assert!(!is_writable_property(
        &bo,
        PropertyIdentifier::PRIORITY_ARRAY
    ));
    assert!(is_writable_property(&bo, PropertyIdentifier::PRESENT_VALUE));
    // Text + common.
    assert!(is_writable_property(&bo, PropertyIdentifier::ACTIVE_TEXT));
    assert!(is_writable_property(&bo, PropertyIdentifier::INACTIVE_TEXT));
    assert!(is_writable_property(
        &bo,
        PropertyIdentifier::OUT_OF_SERVICE
    ));
    // Writable since #222: Clause 12.7 requires the supported Event_Enable
    // value set to include (T, T, T); writability is this stack's route to
    // that minimum, since the detectors default to (F, F, F) and had no
    // other commissioning path.
    assert!(is_writable_property(&bo, PropertyIdentifier::EVENT_ENABLE));
}

/// #270: a BinaryOutput Relinquish_Default write is validated like a
/// commanded Present_Value (BinaryPV 0/1) and — with an all-NULL priority
/// array — Present_Value immediately resolves to the written default.
#[test]
fn bo_relinquish_default_write_recaptures_present_value() {
    let bo = BinaryOutputObject::new(1, "BO-1");
    let mut bo = bo.unwrap();
    assert!(is_writable_property(
        &bo,
        PropertyIdentifier::RELINQUISH_DEFAULT
    ));

    bo.write_property(
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1),
        "with an empty priority array, PV must resolve to the written default"
    );

    // A live command still outranks the default; relinquishing falls back to
    // the written default.
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        Some(8),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(8),
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );

    for value in [PropertyValue::Enumerated(5), PropertyValue::Unsigned(1)] {
        assert!(
            bo.write_property(PropertyIdentifier::RELINQUISH_DEFAULT, None, value, None)
                .is_err()
        );
        assert_eq!(
            bo.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    // The local setter shares the validation.
    assert!(bo.set_relinquish_default(2).is_err());
    bo.set_relinquish_default(1).unwrap();
}
