//! Reliability simulated out of service (Clause 12.61.10), and the
//! Max_Pres_Value floor that keeps Present_Value in range (Clause 12.61.4;
//! #2280). The shared Reliability gate in `reliability_writability_tests.rs`
//! covers the value checks and the application route; these pin what is
//! the Accumulator's own.

use super::AccumulatorObject;
use crate::traits::{BACnetObject, ReliabilityEval};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{PropertyValue, StatusFlags};

fn accumulator() -> AccumulatorObject {
    AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap()
}

fn read(object: &AccumulatorObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn write(object: &mut AccumulatorObject, property: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn set_out_of_service(object: &mut AccumulatorObject, out_of_service: bool) {
    write(
        object,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

fn reliability(object: &AccumulatorObject) -> Reliability {
    let PropertyValue::Enumerated(raw) = read(object, P::RELIABILITY) else {
        panic!("Reliability reads as an Enumerated");
    };
    Reliability::from_raw(raw)
}

fn fault_flag(object: &AccumulatorObject) -> bool {
    let PropertyValue::BitString { data, .. } = read(object, P::STATUS_FLAGS) else {
        panic!("Status_Flags reads as a BitString");
    };
    data[0] & (StatusFlags::FAULT.bits() << 4) != 0
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

#[test]
fn out_of_service_reliability_write_raises_fault_and_return_restores() {
    let mut object = accumulator();
    object.set_reliability(Reliability::NO_SENSOR).unwrap();
    set_out_of_service(&mut object, true);
    let simulated = PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw());
    write(&mut object, P::RELIABILITY, simulated).unwrap();
    assert_eq!(reliability(&object), Reliability::OVER_RANGE);
    assert!(fault_flag(&object));

    // Clearing the simulated fault clears FAULT too.
    let cleared = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    write(&mut object, P::RELIABILITY, cleared).unwrap();
    assert!(!fault_flag(&object));

    // Back in service, the application's NO_SENSOR is in force again.
    set_out_of_service(&mut object, false);
    assert_eq!(reliability(&object), Reliability::NO_SENSOR);
    assert!(fault_flag(&object));
}

#[test]
fn in_service_reliability_write_is_denied_whatever_the_datatype() {
    let mut object = accumulator();
    for value in [
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
        PropertyValue::Unsigned(2),
    ] {
        assert_property_error(
            write(&mut object, P::RELIABILITY, value).unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(reliability(&object), Reliability::NO_FAULT_DETECTED);
    assert!(!fault_flag(&object));
}

#[test]
fn out_of_service_reliability_write_needs_an_enumerated() {
    let mut object = accumulator();
    set_out_of_service(&mut object, true);
    assert_property_error(
        write(&mut object, P::RELIABILITY, PropertyValue::Unsigned(2)).unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(reliability(&object), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn max_pres_value_below_present_value_is_refused() {
    let mut object = accumulator();
    object.set_present_value(500);
    for out_of_service in [false, true] {
        set_out_of_service(&mut object, out_of_service);
        assert_property_error(
            write(&mut object, P::MAX_PRES_VALUE, PropertyValue::Unsigned(499)).unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        // Neither property moves: no clamp, no modulo.
        assert_eq!(
            read(&object, P::MAX_PRES_VALUE),
            PropertyValue::Unsigned(u64::MAX)
        );
        assert_eq!(
            read(&object, P::PRESENT_VALUE),
            PropertyValue::Unsigned(500)
        );
    }
    // The boundary: a maximum equal to the count is accepted.
    write(&mut object, P::MAX_PRES_VALUE, PropertyValue::Unsigned(500)).unwrap();
    assert_eq!(
        read(&object, P::MAX_PRES_VALUE),
        PropertyValue::Unsigned(500)
    );
    assert_eq!(
        read(&object, P::PRESENT_VALUE),
        PropertyValue::Unsigned(500)
    );
}

#[test]
fn shrinking_the_range_goes_through_a_present_value_write() {
    let mut object = accumulator();
    object.set_present_value(500);
    set_out_of_service(&mut object, true);
    write(&mut object, P::PRESENT_VALUE, PropertyValue::Unsigned(40)).unwrap();
    write(&mut object, P::MAX_PRES_VALUE, PropertyValue::Unsigned(99)).unwrap();
    assert_eq!(
        read(&object, P::MAX_PRES_VALUE),
        PropertyValue::Unsigned(99)
    );
    assert_eq!(read(&object, P::PRESENT_VALUE), PropertyValue::Unsigned(40));
}

#[test]
fn application_present_value_is_kept_within_max_pres_value() {
    let mut object = accumulator();
    // The default maximum, u64::MAX, takes any value as it is.
    object.set_present_value(u64::MAX);
    assert_eq!(
        read(&object, P::PRESENT_VALUE),
        PropertyValue::Unsigned(u64::MAX)
    );
    object.set_present_value(0);
    write(&mut object, P::MAX_PRES_VALUE, PropertyValue::Unsigned(99)).unwrap();
    // Above the maximum, the value is stored modulo 100, as the feed rolls.
    object.set_present_value(250);
    assert_eq!(read(&object, P::PRESENT_VALUE), PropertyValue::Unsigned(50));
    object.set_present_value(99);
    assert_eq!(read(&object, P::PRESENT_VALUE), PropertyValue::Unsigned(99));
    object.set_present_value(100);
    assert_eq!(read(&object, P::PRESENT_VALUE), PropertyValue::Unsigned(0));
}
