//! A Trend Log reference with an array index reads that element (#1205).

use super::*;
use crate::analog::AnalogOutputObject;
use bacnet_types::enums::EngineeringUnits;

/// The fixture's Trend Log, retargeted at `property[index]` of AO-1, whose
/// Priority_Array holds 61.5 at priority 8. Returns the log after one poll.
fn polled_with(property: P, index: Option<u32>) -> (ObjectDatabase, ObjectIdentifier) {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    let mut output = AnalogOutputObject::new(1, "AO", EngineeringUnits::NO_UNITS).unwrap();
    output
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Real(61.5),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let ao = output.object_identifier();
    db.add(Box::new(output)).unwrap();
    let mut object = trend(u32::MAX, 16);
    object
        .set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: ao,
            property_identifier: property.to_raw(),
            property_array_index: index,
            device_identifier: None,
        }))
        .unwrap();
    db.add(Box::new(object)).unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    (db, oid)
}

#[test]
fn an_indexed_reference_logs_the_array_element() {
    for (index, expected) in [
        (8, LogDatum::RealValue(61.5)),
        (16, LogDatum::NullValue),
        // Element zero is the array's length.
        (0, LogDatum::UnsignedValue(16)),
    ] {
        let (db, oid) = polled_with(P::PRIORITY_ARRAY, Some(index));
        assert_eq!(last_datum(&db, oid), expected, "index {index}");
    }
}

#[test]
fn an_index_past_the_end_logs_invalid_array_index() {
    for index in [17, u32::MAX] {
        let (db, oid) = polled_with(P::PRIORITY_ARRAY, Some(index));
        assert_eq!(
            last_datum(&db, oid),
            failure(ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX),
            "index {index}"
        );
    }
}

#[test]
fn an_index_on_a_scalar_logs_property_is_not_an_array() {
    let (db, oid) = polled_with(P::PRESENT_VALUE, Some(1));
    assert_eq!(
        last_datum(&db, oid),
        failure(ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );
    // The unindexed read of the same property still logs its value.
    let (db, oid) = polled_with(P::PRESENT_VALUE, None);
    assert_eq!(last_datum(&db, oid), LogDatum::RealValue(61.5));
}

#[test]
fn an_index_that_is_not_unsigned_leaves_the_log_unpolled() {
    use std::sync::atomic::AtomicU32;
    let (mut db, oid, _, _) = fixture(u32::MAX);
    let inner = db.remove(&oid).unwrap().unwrap();
    db.add(Box::new(ConfigurableTrend {
        inner,
        reference: Arc::new(Mutex::new(PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target()),
            PropertyValue::Unsigned(P::PRIORITY_ARRAY.to_raw().into()),
            PropertyValue::Real(1.0),
        ]))),
        mode: Arc::new(AtomicU32::new(0)),
    }))
    .unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 0);
    assert!(!db.trend_poll.0.contains_key(&oid));
}
