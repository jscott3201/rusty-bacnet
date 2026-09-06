use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::loop_obj::LoopObject;
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::schedule::ScheduleObject;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

fn assert_reliability_gate(object: &mut dyn BACnetObject) {
    assert!(
        object.is_writable_property(PropertyIdentifier::RELIABILITY),
        "static capability must advertise Reliability as writable"
    );

    let denied = object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .expect_err("in-service Reliability write must be refused");
    match denied {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / WRITE_ACCESS_DENIED, got {other:?}"),
    }

    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .expect("Out_Of_Service must be writable");

    for invalid in [11, 26, 63, 65_536] {
        let invalid_value = object
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(invalid),
                None,
            )
            .expect_err("reserved or out-of-range Reliability must be refused");
        match invalid_value {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32);
            }
            other => panic!("expected PROPERTY / VALUE_OUT_OF_RANGE, got {other:?}"),
        }
    }
    for valid in [25, 64, 65_535] {
        object
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(valid),
                None,
            )
            .expect("defined or vendor Reliability boundary must be accepted");
    }

    object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .expect("out-of-service Reliability write must succeed");
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .expect("Reliability must read back"),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
    );

    let internal_denied = object
        .set_reliability_internal(Reliability::NO_FAULT_DETECTED.to_raw())
        .expect_err("internal Reliability write must not overwrite client simulation");
    match internal_denied {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / WRITE_ACCESS_DENIED, got {other:?}"),
    }

    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .expect("returning to service must succeed");
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .expect("restored Reliability must read back"),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "returning to service must discard the simulated Reliability"
    );

    let invalid_internal = object
        .set_reliability_internal(65_536)
        .expect_err("internal route must enforce the Reliability datatype");
    match invalid_internal {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / VALUE_OUT_OF_RANGE, got {other:?}"),
    }
    object
        .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
        .expect("internal Reliability write must succeed in service");
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .expect("internally written Reliability must read back"),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
    );
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .expect("redundant in-service write must succeed");
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .expect("Reliability must survive redundant FALSE"),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "a redundant FALSE write must not replay restore logic"
    );

    object
        .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
        .expect("internal evaluator must establish a non-default saved value");
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .expect("entering a second simulation must succeed");
    object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .expect("second client simulation must succeed");
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .expect("leaving a second simulation must succeed");
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .expect("saved non-default Reliability must read back"),
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
        "save and restore must preserve a non-default evaluated value"
    );
}

#[test]
fn reliability_value_boundaries_match_asn1() {
    let boundaries = [11, 25, 26, 63, 64, 65_535, 65_536];
    let actual: Vec<bool> = boundaries
        .into_iter()
        .map(crate::common::is_reliability_value_valid)
        .collect();
    assert_eq!(
        actual,
        vec![false, true, false, false, true, true, false],
        "boundary order is 11, 25, 26, 63, 64, 65535, 65536"
    );
}

/// #252: the write-path predicate must be exactly ALL_NAMED-membership plus
/// the vendor-proprietary range — never a restated literal that can drift
/// from `Reliability`. Sweeps the full BACnetReliability domain (0..=65535
/// plus the first overflow value).
#[test]
fn reliability_predicate_tracks_all_named_plus_vendor_range() {
    let named: std::collections::BTreeSet<u32> = Reliability::ALL_NAMED
        .iter()
        .map(|&(_, r)| r.to_raw())
        .collect();
    for value in 0..=65_536u32 {
        assert_eq!(
            crate::common::is_reliability_value_valid(value),
            named.contains(&value) || (64..=65_535).contains(&value),
            "predicate must equal ALL_NAMED-membership ∪ vendor range at {value}"
        );
    }
    assert!(!crate::common::is_reliability_value_valid(u32::MAX));

    // The next-unnamed value above the enum ceiling flips from rejected to
    // accepted exactly when its addendum constant lands in `Reliability`, so
    // derive the boundary from ALL_NAMED instead of hardcoding 25/26.
    let enum_ceiling = *named
        .iter()
        .filter(|&&v| v < 64)
        .max()
        .expect("named set is non-empty");
    assert!(
        !named.contains(&(enum_ceiling + 1)),
        "test premise: {enum_ceiling} + 1 is not yet named"
    );
    assert!(
        !crate::common::is_reliability_value_valid(enum_ceiling + 1),
        "the value past the enum ceiling must stay rejected until its constant lands"
    );
    // 11 is reserved for a future addendum inside the named span (Clause 21).
    assert!(!named.contains(&11));
    assert!(!crate::common::is_reliability_value_valid(11));
}

#[test]
fn entering_out_of_service_saves_evaluated_reliability() {
    let mut out_of_service = false;
    let mut reliability = Reliability::OVER_RANGE.to_raw();
    let mut saved = None;

    crate::common::write_out_of_service_with_reliability_restore(
        &mut out_of_service,
        &mut reliability,
        &mut saved,
        PropertyIdentifier::OUT_OF_SERVICE,
        &PropertyValue::Boolean(true),
    )
    .expect("Out_Of_Service must be handled")
    .expect("entry must succeed");

    assert!(out_of_service);
    assert_eq!(saved, Some(Reliability::OVER_RANGE.to_raw()));
    assert_eq!(reliability, Reliability::OVER_RANGE.to_raw());
}

#[test]
fn leaving_out_of_service_restores_saved_reliability() {
    let mut out_of_service = true;
    let mut reliability = Reliability::NO_SENSOR.to_raw();
    let mut saved = Some(Reliability::OVER_RANGE.to_raw());

    crate::common::write_out_of_service_with_reliability_restore(
        &mut out_of_service,
        &mut reliability,
        &mut saved,
        PropertyIdentifier::OUT_OF_SERVICE,
        &PropertyValue::Boolean(false),
    )
    .expect("Out_Of_Service must be handled")
    .expect("exit must succeed");

    assert!(!out_of_service);
    assert_eq!(saved, None);
    assert_eq!(reliability, Reliability::OVER_RANGE.to_raw());
}

#[test]
fn leaving_out_of_service_without_saved_value_falls_back_to_no_fault() {
    let mut out_of_service = true;
    let mut reliability = Reliability::NO_SENSOR.to_raw();
    let mut saved = None;

    crate::common::write_out_of_service_with_reliability_restore(
        &mut out_of_service,
        &mut reliability,
        &mut saved,
        PropertyIdentifier::OUT_OF_SERVICE,
        &PropertyValue::Boolean(false),
    )
    .expect("Out_Of_Service must be handled")
    .expect("fallback exit must succeed");

    assert!(!out_of_service);
    assert_eq!(saved, None);
    assert_eq!(reliability, Reliability::NO_FAULT_DETECTED.to_raw());
}

macro_rules! reliability_gate_test {
    ($name:ident, $object:expr) => {
        #[test]
        fn $name() {
            let mut object = $object;
            assert_reliability_gate(&mut object);
        }
    };
}

reliability_gate_test!(
    analog_input_reliability_requires_out_of_service,
    AnalogInputObject::new(1, "AI-1", 62).unwrap()
);
reliability_gate_test!(
    analog_output_reliability_requires_out_of_service,
    AnalogOutputObject::new(1, "AO-1", 62).unwrap()
);
reliability_gate_test!(
    analog_value_reliability_requires_out_of_service,
    AnalogValueObject::new(1, "AV-1", 62).unwrap()
);
reliability_gate_test!(
    binary_input_reliability_requires_out_of_service,
    BinaryInputObject::new(1, "BI-1").unwrap()
);
reliability_gate_test!(
    binary_output_reliability_requires_out_of_service,
    BinaryOutputObject::new(1, "BO-1").unwrap()
);
reliability_gate_test!(
    binary_value_reliability_requires_out_of_service,
    BinaryValueObject::new(1, "BV-1").unwrap()
);
reliability_gate_test!(
    multistate_input_reliability_requires_out_of_service,
    MultiStateInputObject::new(1, "MSI-1", 3).unwrap()
);
reliability_gate_test!(
    multistate_output_reliability_requires_out_of_service,
    MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()
);
reliability_gate_test!(
    multistate_value_reliability_requires_out_of_service,
    MultiStateValueObject::new(1, "MSV-1", 3).unwrap()
);
// Clause 12.17 Table 12-20 lists Reliability O7; footnote 7 requires
// writability when Out_Of_Service is TRUE.
reliability_gate_test!(
    loop_reliability_requires_out_of_service,
    LoopObject::new(1, "LOOP-1", 62).unwrap()
);
// Clause 12.24: the Reliability_Evaluation_Inhibit text anticipates an
// out-of-service client write ("...unless Out_Of_Service is TRUE and an
// alternate value has been written to the Reliability property").
reliability_gate_test!(
    schedule_reliability_requires_out_of_service,
    ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(0.0)).unwrap()
);

fn assert_protocol_error(error: Error, class: ErrorClass, code: ErrorCode) {
    match error {
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } => {
            assert_eq!(actual_class, class.to_raw() as u32);
            assert_eq!(actual_code, code.to_raw() as u32);
        }
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object
        .read_property(property, None)
        .expect("test property must be readable")
}

/// Clients own an Input's simulation while OOS; the application owns its live
/// logical value while in service.
fn assert_present_value_ownership(
    object: &mut dyn BACnetObject,
    application_value: PropertyValue,
    simulated_value: PropertyValue,
    rejected_application_value: PropertyValue,
) {
    let initial = read(object, PropertyIdentifier::PRESENT_VALUE);
    let denied = object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            application_value.clone(),
            None,
        )
        .expect_err("in-service Present_Value write must be refused over the network");
    assert_protocol_error(denied, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(read(object, PropertyIdentifier::PRESENT_VALUE), initial);

    object
        .set_present_value_internal(application_value.clone())
        .expect("the application must supply Present_Value while in service");

    assert_eq!(
        read(object, PropertyIdentifier::PRESENT_VALUE),
        application_value
    );
    assert_eq!(
        read(object, PropertyIdentifier::OUT_OF_SERVICE),
        PropertyValue::Boolean(false),
        "supplying a value must not put the object out of service"
    );

    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .expect("Out_Of_Service must be writable");
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            simulated_value.clone(),
            None,
        )
        .expect("the network may supply an out-of-service simulation value");
    let simulated_reliability = read(object, PropertyIdentifier::RELIABILITY);

    let denied = object
        .set_present_value_internal(rejected_application_value)
        .expect_err("the application must not override an out-of-service value");
    assert_protocol_error(denied, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        read(object, PropertyIdentifier::PRESENT_VALUE),
        simulated_value
    );
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        simulated_reliability,
        "an OOS application rejection must not recompute Reliability"
    );
}

#[test]
fn analog_input_present_value_is_application_supplied() {
    let mut object = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    assert_present_value_ownership(
        &mut object,
        PropertyValue::Real(21.5),
        PropertyValue::Real(72.0),
        PropertyValue::Real(19.0),
    );
}

#[test]
fn binary_input_present_value_is_application_supplied() {
    let mut object = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_present_value_ownership(
        &mut object,
        PropertyValue::Enumerated(1),
        PropertyValue::Enumerated(0),
        PropertyValue::Enumerated(1),
    );
}

#[test]
fn multistate_input_present_value_is_application_supplied() {
    let mut object = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert_present_value_ownership(
        &mut object,
        PropertyValue::Unsigned(2),
        PropertyValue::Unsigned(3),
        PropertyValue::Unsigned(1),
    );
}

#[test]
fn commandable_present_value_fails_closed_by_default() {
    let mut object = AnalogValueObject::new(1, "AV-1", 62).unwrap();

    let error = object
        .set_present_value_internal(PropertyValue::Real(21.5))
        .expect_err("commandable objects must not inherit privileged authority");
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(
        read(&object, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(0.0)
    );
}

fn assert_internal_rejection_is_atomic(
    object: &mut dyn BACnetObject,
    value: PropertyValue,
    code: ErrorCode,
) {
    let present_value = read(object, PropertyIdentifier::PRESENT_VALUE);
    let reliability = read(object, PropertyIdentifier::RELIABILITY);
    let error = object
        .set_present_value_internal(value)
        .expect_err("invalid application Present_Value must be refused");
    assert_protocol_error(error, ErrorClass::PROPERTY, code);
    assert_eq!(
        read(object, PropertyIdentifier::PRESENT_VALUE),
        present_value
    );
    assert_eq!(read(object, PropertyIdentifier::RELIABILITY), reliability);
}

#[test]
fn application_present_value_writes_are_still_validated() {
    let mut analog = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    analog
        .set_present_value_internal(PropertyValue::Real(21.5))
        .unwrap();
    assert_internal_rejection_is_atomic(
        &mut analog,
        PropertyValue::Enumerated(1),
        ErrorCode::INVALID_DATA_TYPE,
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_internal_rejection_is_atomic(
            &mut analog,
            PropertyValue::Real(value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }

    let mut binary = BinaryInputObject::new(1, "BI-1").unwrap();
    binary
        .set_present_value_internal(PropertyValue::Enumerated(1))
        .unwrap();
    assert_internal_rejection_is_atomic(
        &mut binary,
        PropertyValue::Boolean(false),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_internal_rejection_is_atomic(
        &mut binary,
        PropertyValue::Enumerated(2),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );

    let mut multistate = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    multistate
        .set_present_value_internal(PropertyValue::Unsigned(2))
        .unwrap();
    assert_internal_rejection_is_atomic(
        &mut multistate,
        PropertyValue::Enumerated(1),
        ErrorCode::INVALID_DATA_TYPE,
    );
    for value in [0, 4] {
        assert_internal_rejection_is_atomic(
            &mut multistate,
            PropertyValue::Unsigned(value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
}

#[test]
fn multistate_application_updates_preserve_reliability_policy() {
    let mut object = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    object
        .set_present_value_internal(PropertyValue::Unsigned(3))
        .unwrap();
    object.set_number_of_states(2).unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw())
    );
    object
        .set_present_value_internal(PropertyValue::Unsigned(2))
        .unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "a valid application update must synchronously recompute Reliability"
    );

    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object.set_number_of_states(1).unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "inhibit must suppress the shrink-triggered recomputation"
    );
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw())
    );

    object.set_number_of_states(2).unwrap();
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object.set_number_of_states(1).unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "OOS must suppress local reliability evaluation"
    );
    let error = object
        .set_present_value_internal(PropertyValue::Unsigned(1))
        .expect_err("application update must not replace the OOS simulation");
    assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        read(&object, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}
