use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::error::Error;

#[test]
fn decode_full_object_list_sequence() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 5007).unwrap();
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1173).unwrap();
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &PropertyValue::ObjectIdentifier(device)).unwrap();
    encode_property_value(&mut buf, &PropertyValue::ObjectIdentifier(ai)).unwrap();

    let oids = decode_object_identifier_list(&buf);
    assert_eq!(oids.len(), 2);
    assert_eq!(oids[0], device);
    assert_eq!(oids[1], ai);
}

#[test]
fn current_command_priority_accepts_only_unsigned_priorities() {
    for priority in 1u8..=16 {
        let bytes = encoded(&PropertyValue::Unsigned(u64::from(priority)));
        assert_eq!(decode_current_command_priority(&bytes), Some(priority));
    }
    for value in [
        0,
        17,
        255,
        256,
        257,
        272,
        65_537,
        (1u64 << 32) + 1,
        u64::MAX,
    ] {
        let bytes = encoded(&PropertyValue::Unsigned(value));
        assert_eq!(decode_current_command_priority(&bytes), None, "{value}");
    }
    for value in [
        PropertyValue::Null,
        PropertyValue::Signed(1),
        PropertyValue::Enumerated(1),
        PropertyValue::Real(1.0),
    ] {
        assert_eq!(decode_current_command_priority(&encoded(&value)), None);
    }
    assert_eq!(decode_current_command_priority(&[]), None);
    assert_eq!(decode_current_command_priority(&[0x22, 0x01]), None);
}

#[test]
fn object_list_count_is_checked_before_allocation_or_indexed_reads() {
    for count in [0u32, 1, 9_999, 10_000] {
        let (indices, capacity) = object_list_bounds(u64::from(count)).unwrap();
        assert_eq!(indices, count);
        assert_eq!(capacity, usize::try_from(count).unwrap());
        assert!(capacity <= MAX_OBJECT_LIST_ENTRIES);
    }
    for count in [10_001, u64::from(u32::MAX)] {
        assert!(matches!(
            object_list_bounds(count),
            Err(Error::Encoding(message))
                if message == "object-list count exceeds example limit of 10000"
        ));
    }
    for count in [u64::from(u32::MAX) + 1, (1u64 << 32) + 1, u64::MAX] {
        assert!(matches!(
            object_list_bounds(count),
            Err(Error::Encoding(message)) if message == "object-list count exceeds u32"
        ));
    }
}

#[test]
fn priority_array_stops_at_sixteen_before_numbering_or_filtering() {
    for len in [0, 1, 16, 17, 256, 257, 272] {
        let slots = active_priority_array_slots(vec![PropertyValue::Unsigned(42); len]);
        let expected: Vec<_> = (1u8..=16)
            .take(len)
            .map(|priority| (priority, PropertyValue::Unsigned(42)))
            .collect();
        assert_eq!(slots, expected, "array length {len}");
    }

    let mut items = vec![PropertyValue::Null; 272];
    items[0] = PropertyValue::Unsigned(1);
    items[15] = PropertyValue::Unsigned(16);
    items[16] = PropertyValue::Unsigned(17);
    items[255] = PropertyValue::Unsigned(256);
    items[256] = PropertyValue::Unsigned(257);
    assert_eq!(
        active_priority_array_slots(items),
        vec![
            (1, PropertyValue::Unsigned(1)),
            (16, PropertyValue::Unsigned(16)),
        ]
    );
}

#[test]
fn priority_rpm_chunk_ignores_excess_results() {
    for start in [1u8, 9] {
        for len in [0, 1, 8, 9, 256, 257, 272] {
            let results = vec![result(Some(encoded(&PropertyValue::Unsigned(42)))); len];
            let slots = active_priority_chunk(start..=start + 7, &results);
            let expected: Vec<_> = (start..=start + 7)
                .take(len)
                .map(|priority| (priority, PropertyValue::Unsigned(42)))
                .collect();
            assert_eq!(slots, expected, "chunk {start}, result count {len}");
        }
    }
}

#[test]
fn priority_rpm_chunk_preserves_positions_across_missing_and_null_values() {
    let mut results = vec![result(Some(encoded(&PropertyValue::Unsigned(42)))); 9];
    results[0] = result(None);
    results[1] = result(Some(encoded(&PropertyValue::Null)));
    results[2] = result(Some(vec![0x22, 0x01]));
    let expected: Vec<_> = (12u8..=16)
        .map(|priority| (priority, PropertyValue::Unsigned(42)))
        .collect();
    assert_eq!(active_priority_chunk(9..=16, &results), expected);
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

fn result(property_value: Option<Vec<u8>>) -> ReadResultElement {
    ReadResultElement {
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: None,
        property_value,
        error: None,
    }
}
