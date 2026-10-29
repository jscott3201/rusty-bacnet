use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::RemoteError;

fn read(lp: &LoopObject, property: PropertyIdentifier) -> PropertyValue {
    lp.read_property(property, None).unwrap()
}

#[test]
fn python_add_loop_settings_reach_the_read_only_rows() {
    let lp = loop_object(
        1,
        "LOOP-1",
        62,
        LoopSettings {
            controlled_variable_units: Some(64),
            proportional_constant_units: Some(98),
            integral_constant_units: Some(73),
            derivative_constant_units: Some(65_535),
            priority_for_writing: Some(9),
        },
    )
    .unwrap();
    for (property, expected) in [
        (PropertyIdentifier::CONTROLLED_VARIABLE_UNITS, 64),
        (PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS, 98),
        (PropertyIdentifier::INTEGRAL_CONSTANT_UNITS, 73),
        (PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS, 65_535),
    ] {
        assert_eq!(read(&lp, property), PropertyValue::Enumerated(expected));
    }
    assert_eq!(
        read(&lp, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(9)
    );

    // Omitted arguments keep the Loop's defaults.
    let defaults = loop_object(2, "LOOP-2", 62, LoopSettings::default()).unwrap();
    assert_eq!(
        read(&defaults, PropertyIdentifier::CONTROLLED_VARIABLE_UNITS),
        PropertyValue::Enumerated(EngineeringUnits::NO_UNITS.to_raw())
    );
    assert_eq!(
        read(&defaults, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(16)
    );
}

#[test]
fn python_add_loop_settings_are_checked_like_the_rust_setters() {
    let units = |set: fn(&mut LoopSettings)| {
        let mut settings = LoopSettings::default();
        set(&mut settings);
        settings
    };
    let refused = [
        units(|s| s.controlled_variable_units = Some(65_536)),
        units(|s| s.proportional_constant_units = Some(65_536)),
        units(|s| s.integral_constant_units = Some(u32::MAX)),
        units(|s| s.derivative_constant_units = Some(70_000)),
        units(|s| s.priority_for_writing = Some(0)),
        units(|s| s.priority_for_writing = Some(17)),
    ];
    for settings in refused {
        let error = loop_object(1, "LOOP-1", 62, settings).err().unwrap();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::VALUE_OUT_OF_RANGE),
            "{error:?}"
        );
    }
}
