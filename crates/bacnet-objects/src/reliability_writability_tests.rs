use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
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
