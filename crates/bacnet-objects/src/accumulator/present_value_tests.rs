//! A client writes Present_Value only while Out_Of_Service is TRUE (Table
//! 12-79 footnote 1, Clause 12.61.4; #2267): an Unsigned within zero through
//! Max_Pres_Value, from which fed pulses count on once back in service.

use super::AccumulatorObject;
use crate::property_metadata::PropertyWriteCapability;
use crate::traits::{BACnetObject, LocalInput, LocalInputValue};
use bacnet_types::constructed::BACnetPrescale;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

fn accumulator() -> AccumulatorObject {
    AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap()
}

fn present_value(object: &AccumulatorObject) -> PropertyValue {
    object.read_property(P::PRESENT_VALUE, None).unwrap()
}

fn set_out_of_service(object: &mut AccumulatorObject, out_of_service: bool) {
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn write(object: &mut AccumulatorObject, value: PropertyValue) -> Result<(), Error> {
    object.write_property(P::PRESENT_VALUE, None, value, None)
}

fn feed(object: &mut AccumulatorObject, pulses: u64) {
    object
        .apply_local_input(LocalInputValue::Pulses(pulses))
        .unwrap();
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
        "{error:?}"
    );
}

#[test]
fn out_of_service_write_sets_present_value() {
    let mut object = accumulator();
    object.set_present_value(5);
    set_out_of_service(&mut object, true);
    assert!(object.is_writable_property(P::PRESENT_VALUE));
    write(&mut object, PropertyValue::Unsigned(1250)).unwrap();
    assert_eq!(present_value(&object), PropertyValue::Unsigned(1250));
    // The row says when: only out of service.
    let row = object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == P::PRESENT_VALUE)
        .copied()
        .unwrap();
    assert_eq!(
        row.write_capability,
        PropertyWriteCapability::WhenOutOfService
    );
}

#[test]
fn writes_stay_within_max_pres_value() {
    let mut object = accumulator();
    object
        .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(999), None)
        .unwrap();
    set_out_of_service(&mut object, true);
    // Max_Pres_Value itself is the largest value taken.
    write(&mut object, PropertyValue::Unsigned(999)).unwrap();
    assert_eq!(present_value(&object), PropertyValue::Unsigned(999));
    write(&mut object, PropertyValue::Unsigned(0)).unwrap();
    for value in [1000, u64::MAX] {
        assert_property_error(
            write(&mut object, PropertyValue::Unsigned(value)).unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(present_value(&object), PropertyValue::Unsigned(0));
    // With the default Max_Pres_Value every Unsigned fits.
    let mut unbounded = accumulator();
    set_out_of_service(&mut unbounded, true);
    write(&mut unbounded, PropertyValue::Unsigned(u64::MAX)).unwrap();
    assert_eq!(present_value(&unbounded), PropertyValue::Unsigned(u64::MAX));
}

#[test]
fn other_datatypes_are_refused() {
    let mut object = accumulator();
    object.set_present_value(7);
    set_out_of_service(&mut object, true);
    for value in [
        PropertyValue::Real(12.0),
        PropertyValue::Double(12.0),
        PropertyValue::Signed(12),
        PropertyValue::Enumerated(12),
        PropertyValue::Null,
    ] {
        assert_property_error(
            write(&mut object, value).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(present_value(&object), PropertyValue::Unsigned(7));
}

#[test]
fn in_service_writes_are_denied() {
    // In service the input owns the count, whatever the value's datatype
    // or size.
    let mut object = accumulator();
    object.set_present_value(7);
    // Advertised as writable (out of service), but dispatch gates it.
    assert!(object.is_writable_property(P::PRESENT_VALUE));
    for value in [
        PropertyValue::Unsigned(10),
        PropertyValue::Unsigned(u64::MAX),
        PropertyValue::Real(10.0),
    ] {
        assert_property_error(
            write(&mut object, value).unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(present_value(&object), PropertyValue::Unsigned(7));
    // Back in service after a write, the gate closes again.
    set_out_of_service(&mut object, true);
    write(&mut object, PropertyValue::Unsigned(20)).unwrap();
    set_out_of_service(&mut object, false);
    assert_property_error(
        write(&mut object, PropertyValue::Unsigned(30)).unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(present_value(&object), PropertyValue::Unsigned(20));
}

#[test]
fn feed_counts_on_from_a_written_value() {
    let mut object = accumulator();
    object
        .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(99), None)
        .unwrap();
    feed(&mut object, 10);
    set_out_of_service(&mut object, true);
    write(&mut object, PropertyValue::Unsigned(95)).unwrap();
    // Out of service, pulses leave the written value alone.
    feed(&mut object, 3);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(95));
    // Back in service, counting resumes from it, wrapping past 99.
    set_out_of_service(&mut object, false);
    feed(&mut object, 7);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(2));
}

#[test]
fn a_write_clears_the_prescale_remainder() {
    // Two counts per fifteen pulses. The fourteen pulses fed before the
    // write were a share of a step toward the replaced count, so the
    // written value starts with an empty remainder: one more pulse alone
    // doesn't step it, and fifteen more step it by two.
    let mut object = accumulator();
    object.set_prescale(BACnetPrescale {
        multiplier: 2,
        modulo_divide: 15,
    });
    feed(&mut object, 7); // 14 of 15 held
    set_out_of_service(&mut object, true);
    write(&mut object, PropertyValue::Unsigned(40)).unwrap();
    set_out_of_service(&mut object, false);
    feed(&mut object, 1); // 2 of 15: no step
    assert_eq!(present_value(&object), PropertyValue::Unsigned(40));
    feed(&mut object, 14); // 2 + 28 = 30: two steps, none left
    assert_eq!(present_value(&object), PropertyValue::Unsigned(42));
}

#[test]
fn a_refused_write_keeps_the_prescale_remainder() {
    let mut object = accumulator();
    object
        .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(99), None)
        .unwrap();
    object.set_prescale(BACnetPrescale {
        multiplier: 2,
        modulo_divide: 15,
    });
    feed(&mut object, 7); // 14 of 15 held
    set_out_of_service(&mut object, true);
    assert!(write(&mut object, PropertyValue::Unsigned(100)).is_err());
    assert!(write(&mut object, PropertyValue::Real(1.0)).is_err());
    set_out_of_service(&mut object, false);
    feed(&mut object, 1); // 16: one step
    assert_eq!(present_value(&object), PropertyValue::Unsigned(1));
}
