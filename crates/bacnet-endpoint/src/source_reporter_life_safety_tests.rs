use super::*;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::traits::LifeSafetyOperationEffect;
use bacnet_types::enums::LifeSafetyOperation;

#[test]
fn life_safety_outcome_and_error_forward_without_losing_deltas() {
    let mut point = LifeSafetyPointObject::new(1, "wrapped point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut object: Box<dyn BACnetObject> = Box::new(point);
    source_reporter::install(&mut object, false).unwrap();
    let outcome = object
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        .unwrap();
    assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
    assert_eq!(
        outcome.changed_properties,
        vec![
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED
        ]
    );
    let before = [
        read(object.as_ref(), PropertyIdentifier::SILENCED),
        read(object.as_ref(), PropertyIdentifier::OPERATION_EXPECTED),
    ];
    assert!(
        matches!(object.apply_life_safety_operation(LifeSafetyOperation::SILENCE), Err(Error::Protocol {class,code}) if class == ErrorClass::OBJECT.to_raw() as u32 && code == ErrorCode::INVALID_OPERATION_IN_THIS_STATE.to_raw() as u32)
    );
    assert_eq!(
        [
            read(object.as_ref(), PropertyIdentifier::SILENCED),
            read(object.as_ref(), PropertyIdentifier::OPERATION_EXPECTED)
        ],
        before
    );
}

#[test]
fn absent_life_safety_capability_remains_explicitly_unsupported() {
    let mut object: Box<dyn BACnetObject> =
        Box::new(bacnet_objects::analog::AnalogInputObject::new(1, "input", 0).unwrap());
    source_reporter::install(&mut object, false).unwrap();
    assert!(
        matches!(object.apply_life_safety_operation(LifeSafetyOperation::SILENCE), Err(Error::Protocol {class,code}) if class == ErrorClass::OBJECT.to_raw() as u32 && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32)
    );
}
