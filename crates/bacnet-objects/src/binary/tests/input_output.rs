use super::super::*;

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
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // active
        Some(8),
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
    let result = bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2), // invalid -- only 0 or 1
        Some(8),
    );
    assert!(result.is_err());
}

#[test]
fn bv_write_wrong_type_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let result = bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0), // wrong type
        Some(8),
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
    let result = bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        None,
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
    bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        None,
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
    bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
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
    bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
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
    let result = bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        None,
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
    let result = bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(0),
    );
    assert!(result.is_err());
}

#[test]
fn bo_write_with_priority_17_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Priority 17 is invalid (valid range is 1-16)
    let result = bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(17),
    );
    assert!(result.is_err());
}

#[test]
fn bo_write_with_priority_255_rejected() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    // Priority 255 is invalid
    let result = bo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(255),
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
    let props = bi.property_list();
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

// --- Direct PRIORITY_ARRAY writes ---

#[test]
fn bo_direct_priority_array_write_value() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Enumerated(1), // active
        None,
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
fn bo_direct_priority_array_relinquish() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    bo.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
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
