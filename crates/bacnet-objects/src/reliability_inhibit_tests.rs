use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::{BACnetObject, ReliabilityEvaluation};
use bacnet_types::enums::{EventState, PropertyIdentifier, Reliability};
use bacnet_types::primitives::PropertyValue;

const INHIBIT: PropertyIdentifier = PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT;

fn nine_objects() -> Vec<(&'static str, Box<dyn BACnetObject>)> {
    vec![
        (
            "analog-input",
            Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()),
        ),
        (
            "analog-output",
            Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()),
        ),
        (
            "analog-value",
            Box::new(AnalogValueObject::new(1, "AV-1", 62).unwrap()),
        ),
        (
            "binary-input",
            Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        ),
        (
            "binary-output",
            Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        ),
        (
            "binary-value",
            Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        ),
        (
            "multistate-input",
            Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
        ),
        (
            "multistate-output",
            Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
        ),
        (
            "multistate-value",
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        ),
    ]
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn read_reliability(object: &dyn BACnetObject) -> u32 {
    match read(object, PropertyIdentifier::RELIABILITY) {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected Enumerated Reliability, got {other:?}"),
    }
}

fn write_bool(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: bool) {
    object
        .write_property(property, None, PropertyValue::Boolean(value), None)
        .unwrap();
}

fn write_client_reliability(object: &mut dyn BACnetObject, value: u32) {
    object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(value),
            None,
        )
        .unwrap();
}

#[test]
fn all_nine_expose_default_false_boolean_writable_property_and_pics_truth() {
    for (name, mut object) in nine_objects() {
        assert_eq!(
            read(&*object, INHIBIT),
            PropertyValue::Boolean(false),
            "{name}"
        );
        assert!(object.property_list().contains(&INHIBIT), "{name}");
        assert!(
            object.is_writable_property(INHIBIT),
            "{name} PICS writability must match dispatch"
        );
        let wire_list = read(&*object, PropertyIdentifier::PROPERTY_LIST);
        assert!(
            matches!(wire_list, PropertyValue::List(ref values)
                if values.contains(&PropertyValue::Enumerated(INHIBIT.to_raw()))),
            "{name} wire Property_List omitted inhibit"
        );

        assert!(object
            .write_property(INHIBIT, None, PropertyValue::Unsigned(1), None)
            .is_err());
        assert_eq!(
            read(&*object, INHIBIT),
            PropertyValue::Boolean(false),
            "{name}"
        );
        write_bool(&mut *object, INHIBIT, true);
        assert_eq!(
            read(&*object, INHIBIT),
            PropertyValue::Boolean(true),
            "{name}"
        );
        write_bool(&mut *object, INHIBIT, false);
        assert_eq!(
            read(&*object, INHIBIT),
            PropertyValue::Boolean(false),
            "{name}"
        );
    }

    let binary_input = BinaryInputObject::new(2, "BI-metadata").unwrap();
    let row = binary_input
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == INHIBIT)
        .copied()
        .expect("Binary Input canonical metadata must include inhibit");
    assert_eq!(row.conformance, PropertyConformance::Optional);
    assert_eq!(row.write_capability, PropertyWriteCapability::Always);
}

#[test]
fn true_normalizes_existing_fault_and_false_does_not_restore_it_on_all_nine() {
    for (name, mut object) in nine_objects() {
        object
            .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
            .unwrap();
        write_bool(&mut *object, INHIBIT, true);
        assert_eq!(
            read_reliability(&*object),
            Reliability::NO_FAULT_DETECTED.to_raw(),
            "{name} froze the prior fault"
        );
        assert!(object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .is_err());
        assert_eq!(
            read_reliability(&*object),
            Reliability::NO_FAULT_DETECTED.to_raw(),
            "{name} internal setter violated inhibit"
        );
        write_bool(&mut *object, INHIBIT, false);
        assert_eq!(
            read_reliability(&*object),
            Reliability::NO_FAULT_DETECTED.to_raw(),
            "{name} restored a pre-inhibit fault"
        );
    }
}

#[test]
fn inhibited_ai_and_av_range_faults_skip_and_reenable_current_evaluation() {
    let mut ai = AnalogInputObject::new(1, "AI-range", 62).unwrap();
    ai.configure_fault_out_of_range(10.0, 20.0).unwrap();
    ai.set_present_value(21.0);
    assert!(matches!(
        ai.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Changed { new_reliability, .. }
            if new_reliability == Reliability::OVER_RANGE.to_raw()
    ));
    write_bool(&mut ai, INHIBIT, true);
    assert_eq!(
        read_reliability(&ai),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    assert_eq!(
        ai.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Unchanged
    );
    write_bool(&mut ai, INHIBIT, false);
    assert_eq!(
        ai.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
            new_reliability: Reliability::OVER_RANGE.to_raw(),
        },
        "inhibit must not clear the range detector's ownership"
    );

    let mut av = AnalogValueObject::new(1, "AV-range", 62).unwrap();
    av.configure_fault_out_of_range(10.0, 20.0).unwrap();
    av.set_present_value(9.0);
    write_bool(&mut av, INHIBIT, true);
    assert_eq!(
        av.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Unchanged
    );
    assert_eq!(
        read_reliability(&av),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    write_bool(&mut av, INHIBIT, false);
    assert_eq!(
        av.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
            new_reliability: Reliability::UNDER_RANGE.to_raw(),
        }
    );
}

#[test]
fn out_of_service_ordering_without_client_override_always_exposes_zero() {
    for inhibit_first in [false, true] {
        let mut object = BinaryInputObject::new(1, "BI-order").unwrap();
        object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        if inhibit_first {
            write_bool(&mut object, INHIBIT, true);
        }
        write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, true);
        if !inhibit_first {
            write_bool(&mut object, INHIBIT, true);
        }
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, false);
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        write_bool(&mut object, INHIBIT, false);
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
    }
}

#[test]
fn accepted_oos_alternate_is_owned_even_when_same_value_and_restores_by_exit_mode() {
    let mut same_value = BinaryInputObject::new(1, "BI-same").unwrap();
    same_value
        .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
        .unwrap();
    write_bool(&mut same_value, PropertyIdentifier::OUT_OF_SERVICE, true);
    write_client_reliability(&mut same_value, Reliability::NO_SENSOR.to_raw());
    write_bool(&mut same_value, INHIBIT, true);
    write_bool(&mut same_value, PropertyIdentifier::OUT_OF_SERVICE, true);
    assert_eq!(
        read_reliability(&same_value),
        Reliability::NO_SENSOR.to_raw()
    );
    write_bool(&mut same_value, INHIBIT, false);
    write_bool(&mut same_value, PropertyIdentifier::OUT_OF_SERVICE, false);
    assert_eq!(
        read_reliability(&same_value),
        Reliability::NO_SENSOR.to_raw(),
        "non-inhibited exit must restore the evaluated value saved on entry"
    );

    let mut inhibited_exit = BinaryInputObject::new(2, "BI-inhibited-exit").unwrap();
    inhibited_exit
        .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
        .unwrap();
    write_bool(
        &mut inhibited_exit,
        PropertyIdentifier::OUT_OF_SERVICE,
        true,
    );
    write_client_reliability(&mut inhibited_exit, Reliability::NO_SENSOR.to_raw());
    write_bool(&mut inhibited_exit, INHIBIT, true);
    assert_eq!(
        read_reliability(&inhibited_exit),
        Reliability::NO_SENSOR.to_raw()
    );
    write_bool(
        &mut inhibited_exit,
        PropertyIdentifier::OUT_OF_SERVICE,
        false,
    );
    assert_eq!(
        read_reliability(&inhibited_exit),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    write_bool(&mut inhibited_exit, INHIBIT, false);
    assert_eq!(
        read_reliability(&inhibited_exit),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
}

#[test]
fn rejected_oos_reliability_writes_do_not_create_an_override() {
    for rejected in [PropertyValue::Boolean(true), PropertyValue::Enumerated(11)] {
        let mut object = BinaryInputObject::new(1, "BI-invalid").unwrap();
        object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, true);
        assert!(object
            .write_property(PropertyIdentifier::RELIABILITY, None, rejected, None)
            .is_err());
        assert_eq!(read_reliability(&object), Reliability::OVER_RANGE.to_raw());
        write_bool(&mut object, INHIBIT, true);
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw(),
            "rejected Reliability write incorrectly established ownership"
        );
    }
}

#[test]
fn repeated_oos_cycles_clear_old_client_ownership() {
    let mut object = BinaryInputObject::new(1, "BI-cycles").unwrap();
    object
        .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
        .unwrap();
    write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, true);
    write_client_reliability(&mut object, Reliability::NO_SENSOR.to_raw());
    write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, false);
    assert_eq!(read_reliability(&object), Reliability::OVER_RANGE.to_raw());

    write_bool(&mut object, INHIBIT, true);
    for _ in 0..2 {
        write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, true);
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        write_bool(&mut object, PropertyIdentifier::OUT_OF_SERVICE, false);
        assert_eq!(
            read_reliability(&object),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
    }
}

#[test]
fn inhibit_does_not_change_detection_enable_and_recovery_is_reportable() {
    let mut object = AnalogInputObject::new(1, "AI-report", 62).unwrap();
    object
        .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
        .unwrap();
    let fault = object
        .evaluate_intrinsic_reporting()
        .expect("nonzero Reliability must propose FAULT");
    crate::event::commit_test_proposal(&mut object, fault);
    assert_eq!(
        read(&object, PropertyIdentifier::EVENT_STATE),
        PropertyValue::Enumerated(EventState::FAULT.to_raw())
    );

    let detection_enable = read(&object, PropertyIdentifier::EVENT_DETECTION_ENABLE);
    write_bool(&mut object, INHIBIT, true);
    assert_eq!(
        read(&object, PropertyIdentifier::EVENT_DETECTION_ENABLE),
        detection_enable
    );
    let recovery = object
        .evaluate_intrinsic_reporting()
        .expect("Reliability normalization must remain visible to intrinsic reporting");
    assert_eq!(recovery.change.from, EventState::FAULT);
    assert_eq!(recovery.change.to, EventState::NORMAL);
}
