use super::super::*;
use crate::traits::{BACnetObject, ReliabilityEvaluation};
use bacnet_types::enums::{ErrorClass, ErrorCode, Reliability};
use bacnet_types::primitives::StatusFlags;

fn reliability(object: &dyn BACnetObject) -> u32 {
    match object
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
    {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected Enumerated Reliability, got {other:?}"),
    }
}

fn assert_changed(
    result: Result<ReliabilityEvaluation, Error>,
    old_reliability: Reliability,
    new_reliability: Reliability,
) {
    assert_eq!(
        result.unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: old_reliability.to_raw(),
            new_reliability: new_reliability.to_raw(),
        }
    );
}

fn assert_unchanged(result: Result<ReliabilityEvaluation, Error>) {
    assert_eq!(result.unwrap(), ReliabilityEvaluation::Unchanged);
}

fn assert_unknown_property(result: Result<PropertyValue, Error>) {
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
    ));
}

fn assert_write_denied(result: Result<(), Error>) {
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
    ));
}

fn assert_fault_flag(object: &dyn BACnetObject, expected: bool) {
    let PropertyValue::BitString { unused_bits, data } = object
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .unwrap()
    else {
        panic!("expected Status_Flags BitString");
    };
    assert_eq!(unused_bits, 4);
    assert_eq!(data[0] & (StatusFlags::FAULT.bits() << 4) != 0, expected);
}

fn assert_fault_properties(object: &dyn BACnetObject, expected: bool) {
    for property in [
        PropertyIdentifier::FAULT_LOW_LIMIT,
        PropertyIdentifier::FAULT_HIGH_LIMIT,
    ] {
        assert_eq!(object.property_list().contains(&property), expected);
    }
    let PropertyValue::List(wire_list) = object
        .read_property(PropertyIdentifier::PROPERTY_LIST, None)
        .unwrap()
    else {
        panic!("expected Property_List List");
    };
    for property in [
        PropertyIdentifier::FAULT_LOW_LIMIT,
        PropertyIdentifier::FAULT_HIGH_LIMIT,
    ] {
        assert_eq!(
            wire_list.contains(&PropertyValue::Enumerated(property.to_raw())),
            expected
        );
    }
}

macro_rules! assert_property_surface {
    ($object:expr) => {{
        let mut object = $object;
        assert_unknown_property(object.read_property(PropertyIdentifier::FAULT_LOW_LIMIT, None));
        assert_unknown_property(object.read_property(PropertyIdentifier::FAULT_HIGH_LIMIT, None));
        assert_fault_properties(&object, false);

        object.configure_fault_out_of_range(-5.0, 25.0).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::FAULT_LOW_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(-5.0)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::FAULT_HIGH_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(25.0)
        );
        assert_fault_properties(&object, true);
        for property in [
            PropertyIdentifier::FAULT_LOW_LIMIT,
            PropertyIdentifier::FAULT_HIGH_LIMIT,
        ] {
            assert!(!object.is_writable_property(property));
            assert_write_denied(object.write_property(
                property,
                None,
                PropertyValue::Real(1.0),
                None,
            ));
        }

        object.configure_fault_out_of_range(3.0, 3.0).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::FAULT_LOW_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(3.0)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::FAULT_HIGH_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(3.0)
        );
    }};
}

#[test]
fn ai_and_av_fault_limit_properties_are_atomic_optional_and_read_only() {
    assert_property_surface!(AnalogInputObject::new(1, "AI-1", 62).unwrap());
    assert_property_surface!(AnalogValueObject::new(1, "AV-1", 62).unwrap());
}

macro_rules! assert_invalid_configuration_is_atomic {
    ($object:expr) => {{
        let mut object = $object;
        let invalid_limits = [
            (f32::NAN, 1.0),
            (1.0, f32::NAN),
            (f32::INFINITY, 1.0),
            (f32::NEG_INFINITY, 1.0),
            (1.0, f32::INFINITY),
            (1.0, f32::NEG_INFINITY),
            (2.0, 1.0),
        ];

        for (low, high) in invalid_limits {
            assert!(object.configure_fault_out_of_range(low, high).is_err());
            assert_unknown_property(
                object.read_property(PropertyIdentifier::FAULT_LOW_LIMIT, None),
            );
            assert_unknown_property(
                object.read_property(PropertyIdentifier::FAULT_HIGH_LIMIT, None),
            );
            assert_fault_properties(&object, false);
            assert_eq!(
                reliability(&object),
                Reliability::NO_FAULT_DETECTED.to_raw()
            );
        }

        object.configure_fault_out_of_range(10.0, 20.0).unwrap();
        object.set_present_value(21.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::OVER_RANGE,
        );
        for (low, high) in invalid_limits {
            assert!(object.configure_fault_out_of_range(low, high).is_err());
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::FAULT_LOW_LIMIT, None)
                    .unwrap(),
                PropertyValue::Real(10.0)
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::FAULT_HIGH_LIMIT, None)
                    .unwrap(),
                PropertyValue::Real(20.0)
            );
            assert_fault_properties(&object, true);
            assert_eq!(reliability(&object), Reliability::OVER_RANGE.to_raw());
        }

        object.set_present_value(20.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::OVER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );
    }};
}

#[test]
fn invalid_ai_and_av_configuration_changes_no_public_or_private_state() {
    assert_invalid_configuration_is_atomic!(AnalogInputObject::new(1, "AI-1", 62).unwrap());
    assert_invalid_configuration_is_atomic!(AnalogValueObject::new(1, "AV-1", 62).unwrap());
}

#[test]
fn configured_ai_evaluator_enters_under_range_from_no_fault() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.configure_fault_out_of_range(10.0, 20.0).unwrap();
    ai.set_present_value(9.0);

    assert_changed(
        ai.evaluate_reliability_internal(),
        Reliability::NO_FAULT_DETECTED,
        Reliability::UNDER_RANGE,
    );
}

macro_rules! assert_strict_range_transitions {
    ($object:expr) => {{
        let mut object = $object;
        object.configure_fault_out_of_range(10.0, 20.0).unwrap();

        object.set_present_value(10.0);
        assert_unchanged(object.evaluate_reliability_internal());
        object.set_present_value(f32::from_bits(10.0f32.to_bits() - 1));
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::UNDER_RANGE,
        );
        assert_fault_flag(&object, true);
        object.set_present_value(10.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::UNDER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );
        assert_fault_flag(&object, false);

        object.set_present_value(20.0);
        assert_unchanged(object.evaluate_reliability_internal());
        object.set_present_value(f32::from_bits(20.0f32.to_bits() + 1));
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::OVER_RANGE,
        );
        object.set_present_value(20.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::OVER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );

        object.set_present_value(9.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::UNDER_RANGE,
        );
        object.set_present_value(21.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::UNDER_RANGE,
            Reliability::OVER_RANGE,
        );
        object.set_present_value(15.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::OVER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );

        object.configure_fault_out_of_range(10.0, 10.0).unwrap();
        object.set_present_value(10.0);
        assert_unchanged(object.evaluate_reliability_internal());
        object.set_present_value(9.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::UNDER_RANGE,
        );
        object.set_present_value(11.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::UNDER_RANGE,
            Reliability::OVER_RANGE,
        );
        object.set_present_value(10.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::OVER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );
    }};
}

#[test]
fn ai_and_av_use_strict_entry_inclusive_recovery_and_no_deadband() {
    assert_strict_range_transitions!(AnalogInputObject::new(1, "AI-1", 62).unwrap());
    assert_strict_range_transitions!(AnalogValueObject::new(1, "AV-1", 62).unwrap());
}

macro_rules! assert_first_stage_precedence {
    ($object:expr) => {{
        let mut object = $object;
        object.configure_fault_out_of_range(10.0, 20.0).unwrap();

        object.set_present_value(21.0);
        object
            .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
            .unwrap();
        assert_unchanged(object.evaluate_reliability_internal());
        assert_eq!(reliability(&object), Reliability::NO_SENSOR.to_raw());

        object
            .set_reliability_internal(Reliability::UNDER_RANGE.to_raw())
            .unwrap();
        assert_unchanged(object.evaluate_reliability_internal());
        assert_eq!(reliability(&object), Reliability::UNDER_RANGE.to_raw());

        object.set_present_value(9.0);
        object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        assert_unchanged(object.evaluate_reliability_internal());
        assert_eq!(reliability(&object), Reliability::OVER_RANGE.to_raw());

        object
            .set_reliability_internal(Reliability::NO_FAULT_DETECTED.to_raw())
            .unwrap();
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::UNDER_RANGE,
        );

        assert!(object.set_reliability_internal(11).is_err());
        object.set_present_value(10.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::UNDER_RANGE,
            Reliability::NO_FAULT_DETECTED,
        );

        object.set_present_value(21.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::OVER_RANGE,
        );
        object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        object.set_present_value(20.0);
        assert_unchanged(object.evaluate_reliability_internal());
        assert_eq!(reliability(&object), Reliability::OVER_RANGE.to_raw());

        object
            .set_reliability_internal(Reliability::NO_FAULT_DETECTED.to_raw())
            .unwrap();
        object.set_present_value(9.0);
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::NO_FAULT_DETECTED,
            Reliability::UNDER_RANGE,
        );
        object.configure_fault_out_of_range(0.0, 5.0).unwrap();
        assert_changed(
            object.evaluate_reliability_internal(),
            Reliability::UNDER_RANGE,
            Reliability::OVER_RANGE,
        );
    }};
}

#[test]
fn ai_and_av_preserve_first_stage_values_and_clear_only_owned_faults() {
    assert_first_stage_precedence!(AnalogInputObject::new(1, "AI-1", 62).unwrap());
    assert_first_stage_precedence!(AnalogValueObject::new(1, "AV-1", 62).unwrap());
}

#[test]
fn ai_out_of_service_simulation_preserves_range_fault_ownership() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.configure_fault_out_of_range(10.0, 20.0).unwrap();
    ai.set_present_value(21.0);
    assert_changed(
        ai.evaluate_reliability_internal(),
        Reliability::NO_FAULT_DETECTED,
        Reliability::OVER_RANGE,
    );

    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(15.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        None,
    )
    .unwrap();

    assert_unchanged(ai.evaluate_reliability_internal());
    assert_eq!(reliability(&ai), Reliability::NO_SENSOR.to_raw());
    assert_eq!(
        ai.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(15.0)
    );

    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(reliability(&ai), Reliability::OVER_RANGE.to_raw());
    assert_changed(
        ai.evaluate_reliability_internal(),
        Reliability::OVER_RANGE,
        Reliability::NO_FAULT_DETECTED,
    );
}

#[test]
fn av_evaluates_resolved_priority_value_without_mutating_priority_array() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.configure_fault_out_of_range(10.0, 20.0).unwrap();
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(16),
    )
    .unwrap();
    let priority_before = av
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    assert_changed(
        av.evaluate_reliability_internal(),
        Reliability::NO_FAULT_DETECTED,
        Reliability::UNDER_RANGE,
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
            .unwrap(),
        priority_before
    );

    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(25.0),
        Some(8),
    )
    .unwrap();
    let priority_before = av
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    assert_changed(
        av.evaluate_reliability_internal(),
        Reliability::UNDER_RANGE,
        Reliability::OVER_RANGE,
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
            .unwrap(),
        priority_before
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        PropertyValue::Real(25.0)
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(16))
            .unwrap(),
        PropertyValue::Real(5.0)
    );

    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_changed(
        av.evaluate_reliability_internal(),
        Reliability::OVER_RANGE,
        Reliability::UNDER_RANGE,
    );
}

#[test]
fn analog_output_has_no_fault_out_of_range_surface_or_evaluator() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    for property in [
        PropertyIdentifier::FAULT_LOW_LIMIT,
        PropertyIdentifier::FAULT_HIGH_LIMIT,
    ] {
        assert_unknown_property(ao.read_property(property, None));
        assert!(!ao.property_list().contains(&property));
        assert!(!ao.is_writable_property(property));
        assert_write_denied(ao.write_property(property, None, PropertyValue::Real(1.0), None));
    }
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(100.0),
        Some(8),
    )
    .unwrap();
    assert_unchanged(ao.evaluate_reliability_internal());
    assert_eq!(reliability(&ao), Reliability::NO_FAULT_DETECTED.to_raw());
}
