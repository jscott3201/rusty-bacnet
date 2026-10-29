//! Lighting Output Present_Value levels between off and the dimmest on level
//! (#1385; Clause 12.54 and 12.54.4).
//!
//! A commanded level above 0.0 and below 1.0 enters its priority slot as 1.0,
//! so the slot, Present_Value and Tracking_Value all read 1.0. 0.0 and 1.0 to
//! 100.0 are stored as written, and a level outside 0.0 to 100.0 is refused
//! with VALUE_OUT_OF_RANGE and changes nothing, but for the blink-warn values
//! -1.0, -2.0 and -3.0 (`warn_tests`).

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const PA: PropertyIdentifier = PropertyIdentifier::PRIORITY_ARRAY;
const TV: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;
const RD: PropertyIdentifier = PropertyIdentifier::RELINQUISH_DEFAULT;

/// The smallest positive REAL, a subnormal just above 0.0.
const JUST_ABOVE_OFF: f32 = f32::from_bits(1);

fn real(object: &LightingOutputObject, property: PropertyIdentifier, index: Option<u32>) -> f32 {
    match object.read_property(property, index).unwrap() {
        PropertyValue::Real(value) => value,
        other => panic!("{property:?}[{index:?}] read {other:?}"),
    }
}

/// Present_Value, Priority_Array[8] and Tracking_Value, as bits so that a
/// level stored as written is told apart from a nearby one.
fn levels_at_8(object: &LightingOutputObject) -> [u32; 3] {
    [
        real(object, PV, None).to_bits(),
        real(object, PA, Some(8)).to_bits(),
        real(object, TV, None).to_bits(),
    ]
}

fn command(object: &mut LightingOutputObject, value: PropertyValue, priority: u8) {
    object
        .write_property_from(
            PV,
            None,
            value,
            Some(priority),
            &crate::command_source::test_origin(),
        )
        .unwrap();
}

fn assert_out_of_range(result: Result<(), Error>) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE);
        }
        other => panic!("expected PROPERTY / VALUE_OUT_OF_RANGE, got {other:?}"),
    }
}

#[test]
fn lighting_output_present_value_below_one_percent_is_stored_as_one_percent() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    for level in [
        JUST_ABOVE_OFF,
        f32::MIN_POSITIVE,
        0.001,
        0.5,
        1.0f32.next_down(),
    ] {
        command(&mut lo, PropertyValue::Real(level), 8);
        assert_eq!(levels_at_8(&lo), [1.0f32.to_bits(); 3], "{level:e}");
        assert_eq!(
            lo.read_property(PropertyIdentifier::CURRENT_COMMAND_PRIORITY, None)
                .unwrap(),
            PropertyValue::Unsigned(8)
        );
        command(&mut lo, PropertyValue::Null, 8);
        assert_eq!(real(&lo, PV, None), 0.0);
    }
}

#[test]
fn lighting_output_present_value_off_and_one_percent_up_are_stored_as_written() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    for level in [
        0.0,
        1.0,
        1.0f32.next_up(),
        50.0,
        100.0f32.next_down(),
        100.0,
    ] {
        command(&mut lo, PropertyValue::Real(level), 8);
        assert_eq!(levels_at_8(&lo), [level.to_bits(); 3], "{level:e}");
    }
}

#[test]
fn lighting_output_present_value_negative_zero_is_stored_as_off() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    command(&mut lo, PropertyValue::Real(-0.0), 8);
    assert_eq!(levels_at_8(&lo), [0.0f32.to_bits(); 3]);
    lo.set_relinquish_default(-0.0).unwrap();
    assert_eq!(real(&lo, RD, None).to_bits(), 0.0f32.to_bits());
}

#[test]
fn lighting_output_present_value_outside_the_range_is_refused_and_changes_nothing() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    command(&mut lo, PropertyValue::Real(0.5), 8);
    // Only -1.0, -2.0 and -3.0 below 0.0 mean anything: the warn commands
    // (#1384, `warn_tests`). Their neighbours are out of range.
    for level in [
        -JUST_ABOVE_OFF,
        -0.5,
        1.0f32.next_up().copysign(-1.0),
        -1.5,
        3.0f32.next_up().copysign(-1.0),
        -4.0,
        100.0f32.next_up(),
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        assert_out_of_range(lo.write_property_from(
            PV,
            None,
            PropertyValue::Real(level),
            Some(8),
            &crate::command_source::test_origin(),
        ));
        assert_eq!(levels_at_8(&lo), [1.0f32.to_bits(); 3], "{level:e}");
    }
}

#[test]
fn lighting_output_relinquish_falls_back_through_a_one_percent_slot() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    command(&mut lo, PropertyValue::Real(0.5), 8);
    command(&mut lo, PropertyValue::Real(40.0), 4);
    assert_eq!([real(&lo, PV, None), real(&lo, TV, None)], [40.0; 2]);
    command(&mut lo, PropertyValue::Null, 4);
    assert_eq!(lo.read_property(PA, Some(4)).unwrap(), PropertyValue::Null);
    assert_eq!(levels_at_8(&lo), [1.0f32.to_bits(); 3]);
    command(&mut lo, PropertyValue::Null, 8);
    assert_eq!(lo.read_property(PA, Some(8)).unwrap(), PropertyValue::Null);
    assert_eq!([real(&lo, PV, None), real(&lo, TV, None)], [0.0; 2]);
}

#[test]
fn lighting_output_relinquish_default_below_one_percent_is_stored_as_one_percent() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_relinquish_default(0.5).unwrap();
    assert_eq!(real(&lo, RD, None), 1.0);
    assert_eq!([real(&lo, PV, None), real(&lo, TV, None)], [1.0; 2]);
    lo.set_relinquish_default(0.0).unwrap();
    assert_eq!([real(&lo, RD, None), real(&lo, PV, None)], [0.0; 2]);
    lo.write_property(RD, None, PropertyValue::Real(JUST_ABOVE_OFF), None)
        .unwrap();
    assert_eq!([real(&lo, RD, None), real(&lo, PV, None)], [1.0; 2]);
    lo.write_property(RD, None, PropertyValue::Real(1.0f32.next_up()), None)
        .unwrap();
    assert_eq!(real(&lo, RD, None), 1.0f32.next_up());
    assert_out_of_range(lo.write_property(RD, None, PropertyValue::Real(-JUST_ABOVE_OFF), None));
    assert_eq!(real(&lo, RD, None), 1.0f32.next_up());
}
