use super::*;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::traits::LifeSafetyOperationEffect;
use bacnet_types::enums::{EngineeringUnits, LifeSafetyOperation, LifeSafetyState};
use bacnet_types::error::RemoteError;

#[test]
fn life_safety_outcome_and_error_forward_without_losing_deltas() {
    let mut point = LifeSafetyPointObject::new(1, "wrapped point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut object: Box<dyn BACnetObject> = Box::new(point);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    let outcome = object
        .life_safety()
        .unwrap()
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
        matches!(object.life_safety().unwrap().apply_life_safety_operation(LifeSafetyOperation::SILENCE), Err(Error::Remote(RemoteError::Error {class,code})) if class == ErrorClass::OBJECT && code == ErrorCode::INVALID_OPERATION_IN_THIS_STATE)
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
    let mut object: Box<dyn BACnetObject> = Box::new(
        bacnet_objects::analog::AnalogInputObject::new(1, "input", EngineeringUnits::SQUARE_METERS)
            .unwrap(),
    );
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    // The wrapper lends what the input lends: no Life Safety capability.
    assert!(object.life_safety().is_none());
}

#[test]
fn life_safety_application_values_forward_through_the_wrapper() {
    let mut object: Box<dyn BACnetObject> =
        Box::new(LifeSafetyPointObject::new(1, "wrapped point").unwrap());
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    let alarm = PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw());
    let pre_alarm = PropertyValue::Enumerated(LifeSafetyState::PRE_ALARM.to_raw());
    object
        .local_input()
        .unwrap()
        .apply_local_input(bacnet_objects::traits::LocalInputValue::present_value(
            alarm.clone(),
        ))
        .unwrap();
    object
        .local_input()
        .unwrap()
        .apply_local_input(bacnet_objects::traits::LocalInputValue::TrackingValue(
            pre_alarm.clone(),
        ))
        .unwrap();
    assert_eq!(
        read(object.as_ref(), PropertyIdentifier::PRESENT_VALUE),
        alarm
    );
    assert_eq!(
        read(object.as_ref(), PropertyIdentifier::TRACKING_VALUE),
        pre_alarm
    );
    // The wrapped object's range check answers, not the trait default.
    assert!(
        matches!(object.local_input().unwrap().apply_local_input(bacnet_objects::traits::LocalInputValue::TrackingValue(PropertyValue::Enumerated(35))), Err(Error::Remote(RemoteError::Error {class,code})) if class == ErrorClass::PROPERTY && code == ErrorCode::VALUE_OUT_OF_RANGE)
    );
    assert_eq!(
        read(object.as_ref(), PropertyIdentifier::TRACKING_VALUE),
        pre_alarm
    );
}
