use super::*;

#[test]
fn command_source_private_nonfinite_av_fault_state_remains_inert_until_valid_command() {
    let mut object = AnalogValueObject::new(
        1,
        "private invalid state",
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    object.configure_fault_out_of_range(10.0, 20.0).unwrap();
    object.set_relinquish_default(9.0).unwrap();
    object.evaluate_reliability_internal().unwrap();
    // This state is unreachable through the public finite-value APIs. Inject
    // it here to retain the evaluator's defensive test without an unsourced setter.
    object.present_value = f32::INFINITY;
    for _ in 0..2 {
        assert!(
            matches!(object.evaluate_reliability_internal(), Err(Error::Protocol { code, .. }) if code == bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::Reliability::UNDER_RANGE.to_raw())
        );
    }
    object
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(21.0),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(matches!(
        object.evaluate_reliability_internal().unwrap(),
        crate::traits::ReliabilityEvaluation::Changed { .. }
    ));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(bacnet_types::enums::Reliability::OVER_RANGE.to_raw())
    );
}
