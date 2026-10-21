use super::*;

#[test]
fn loop_read_defaults() {
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::SETPOINT, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::PROPORTIONAL_CONSTANT, None)
            .unwrap(),
        PropertyValue::Real(1.0)
    );
}

#[test]
fn loop_write_pid_constants() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.write_property(
        PropertyIdentifier::SETPOINT,
        None,
        PropertyValue::Real(72.0),
        None,
    )
    .unwrap();
    lo.write_property(
        PropertyIdentifier::PROPORTIONAL_CONSTANT,
        None,
        PropertyValue::Real(2.5),
        None,
    )
    .unwrap();
    lo.write_property(
        PropertyIdentifier::INTEGRAL_CONSTANT,
        None,
        PropertyValue::Real(0.1),
        None,
    )
    .unwrap();
    lo.write_property(
        PropertyIdentifier::DERIVATIVE_CONSTANT,
        None,
        PropertyValue::Real(0.05),
        None,
    )
    .unwrap();

    assert_eq!(
        lo.read_property(PropertyIdentifier::SETPOINT, None)
            .unwrap(),
        PropertyValue::Real(72.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::PROPORTIONAL_CONSTANT, None)
            .unwrap(),
        PropertyValue::Real(2.5)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::INTEGRAL_CONSTANT, None)
            .unwrap(),
        PropertyValue::Real(0.1)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::DERIVATIVE_CONSTANT, None)
            .unwrap(),
        PropertyValue::Real(0.05)
    );
}

#[test]
fn loop_set_present_value() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_present_value(55.0);
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(55.0)
    );
}

#[test]
fn loop_read_object_type() {
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let val = lo
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(ObjectType::LOOP.to_raw()));
}

#[test]
fn loop_write_wrong_type_rejected() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let result = lo.write_property(
        PropertyIdentifier::SETPOINT,
        None,
        PropertyValue::Unsigned(72),
        None,
    );
    assert!(result.is_err());
}

// --- #985: COV_Increment, COV report contents, Present_Value writability ---

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY && code == expected),
        "expected PROPERTY/{expected:?}, got {error:?}"
    );
}

fn read(lo: &LoopObject, property: PropertyIdentifier) -> PropertyValue {
    lo.read_property(property, None).unwrap()
}

fn set_out_of_service(lo: &mut LoopObject, value: bool) {
    lo.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(value),
        None,
    )
    .unwrap();
}

#[test]
fn loop_cov_increment_is_writable_validated_and_drives_cov() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::COV_INCREMENT),
        PropertyValue::Real(0.0)
    );
    assert_eq!(lo.cov_increment(), Some(0.0));
    assert!(lo.is_writable_property(PropertyIdentifier::COV_INCREMENT));

    lo.write_property(
        PropertyIdentifier::COV_INCREMENT,
        None,
        PropertyValue::Real(2.5),
        None,
    )
    .unwrap();
    assert_eq!(lo.cov_increment(), Some(2.5));

    for (value, code) in [
        (PropertyValue::Real(-0.5), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Unsigned(3), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_property_error(
            lo.write_property(PropertyIdentifier::COV_INCREMENT, None, value, None)
                .unwrap_err(),
            code,
        );
    }
    assert_eq!(
        read(&lo, PropertyIdentifier::COV_INCREMENT),
        PropertyValue::Real(2.5),
        "a refused write keeps the increment"
    );
}

#[test]
fn loop_present_value_is_writable_only_out_of_service() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_present_value(10.0);
    // Writable for PICS purposes; the state gate decides each request.
    assert!(lo.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    assert_property_error(
        lo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(10.0)
    );

    set_out_of_service(&mut lo, true);
    lo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(42.0),
        None,
    )
    .unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(42.0)
    );
    for (value, code) in [
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Unsigned(1), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_property_error(
            lo.write_property(PropertyIdentifier::PRESENT_VALUE, None, value, None)
                .unwrap_err(),
            code,
        );
    }
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(42.0)
    );

    set_out_of_service(&mut lo, false);
    assert_property_error(
        lo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(7.0),
            None,
        )
        .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn loop_application_present_value_is_refused_out_of_service() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_present_value_internal(PropertyValue::Real(12.0))
        .unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(12.0)
    );

    set_out_of_service(&mut lo, true);
    lo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(99.0),
        None,
    )
    .unwrap();
    assert_property_error(
        lo.set_present_value_internal(PropertyValue::Real(13.0))
            .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(99.0),
        "the client's simulated output survives the algorithm"
    );

    set_out_of_service(&mut lo, false);
    assert_property_error(
        lo.set_present_value_internal(PropertyValue::Real(f32::INFINITY))
            .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    lo.set_present_value_internal(PropertyValue::Real(14.0))
        .unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(14.0)
    );
}

#[test]
fn loop_controlled_variable_value_is_application_fed_and_read_only() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        PropertyValue::Real(0.0)
    );
    lo.set_controlled_variable_value(21.5);
    assert_eq!(
        read(&lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        PropertyValue::Real(21.5)
    );
    assert!(!lo.is_writable_property(PropertyIdentifier::CONTROLLED_VARIABLE_VALUE));
    for out_of_service in [false, true] {
        set_out_of_service(&mut lo, out_of_service);
        assert_property_error(
            lo.write_property(
                PropertyIdentifier::CONTROLLED_VARIABLE_VALUE,
                None,
                PropertyValue::Real(1.0),
                None,
            )
            .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

// --- #1063: the application route a running server uses ---

#[test]
fn loop_controlled_variable_value_hook_takes_finite_reals_in_and_out_of_service() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (out_of_service, measured) in [(false, 20.5), (true, 19.0), (false, -4.25)] {
        set_out_of_service(&mut lo, out_of_service);
        lo.set_controlled_variable_value_internal(PropertyValue::Real(measured))
            .unwrap();
        assert_eq!(
            read(&lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
            PropertyValue::Real(measured)
        );
    }
    for (value, code) in [
        (PropertyValue::Double(1.0), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_property_error(
            lo.set_controlled_variable_value_internal(value)
                .unwrap_err(),
            code,
        );
    }
    assert_eq!(
        read(&lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        PropertyValue::Real(-4.25),
        "a refused value keeps the measurement"
    );
}

#[test]
fn controlled_variable_value_hook_is_refused_by_other_objects() {
    let mut av =
        crate::analog::AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS)
            .unwrap();
    let error = av
        .set_controlled_variable_value_internal(PropertyValue::Real(1.0))
        .unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::OBJECT
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        "{error:?}"
    );
}

#[test]
fn loop_cov_reports_setpoint_and_controlled_variable_value() {
    use crate::traits::CovReportedProperty::Value;
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        lo.cov_reported_properties(),
        [
            Value(PropertyIdentifier::SETPOINT),
            Value(PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        ]
    );
    let list = lo.property_list();
    for reported in lo.cov_reported_properties() {
        assert!(list.contains(&reported.property()));
        assert!(!reported.triggers(), "Table 13-1 reports them only");
    }
}
