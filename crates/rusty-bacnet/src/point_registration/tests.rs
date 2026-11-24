//! The shared checks and build of a point (#2711).

use super::*;
use pyo3::Python;

fn analog_input(instance: u32, name: &str, present_value: f32) -> Registration {
    Registration::new(
        instance,
        name,
        RegistrationKind::AnalogInput {
            units: 62,
            present_value,
        },
        None,
        AlgorithmKeywords::default(),
    )
}

/// The text a Python exception carries, and whether it is a ValueError.
fn refusal(error: PyErr) -> (bool, String) {
    Python::attach(|py| (error.is_instance_of::<PyValueError>(py), error.to_string()))
}

#[test]
fn each_object_is_compared_identifier_first() {
    Python::initialize();
    let taken = [
        (ObjectType::DEVICE, 9, "Host"),
        (ObjectType::ANALOG_INPUT, 1, "Kept"),
    ];
    let check = |instance, name| check_unique(ObjectType::ANALOG_INPUT, instance, name, taken);
    for (instance, name, expected) in [
        (
            1,
            "Kept",
            "duplicate object identifier: ObjectType::ANALOG_INPUT 1",
        ),
        (
            1,
            "Fresh",
            "duplicate object identifier: ObjectType::ANALOG_INPUT 1",
        ),
        (2, "Kept", "duplicate object name: Kept"),
        // The Device comes first, so its name clashes before the point's
        // identifier is reached.
        (1, "Host", "duplicate object name: Host"),
    ] {
        let (value_error, text) = refusal(check(instance, name).unwrap_err());
        assert!(value_error, "{text}");
        assert_eq!(text, format!("ValueError: {expected}"));
    }
    // Another type may share an instance.
    check(9, "Fresh").unwrap();
}

#[test]
fn an_instance_no_object_may_have_is_refused() {
    Python::initialize();
    for instance in [ObjectIdentifier::WILDCARD_INSTANCE, 4_194_304, u32::MAX] {
        let (value_error, text) =
            refusal(check_identifier(ObjectType::BINARY_VALUE, instance).unwrap_err());
        assert!(!value_error, "{instance}: {text}");
    }
    check_identifier(ObjectType::BINARY_VALUE, 4_194_302).unwrap();
}

#[test]
fn an_analog_input_value_must_be_finite() {
    Python::initialize();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let error = analog_input(1, "AI", value)
            .build(Storage::NONE)
            .map(|_| ())
            .unwrap_err();
        Python::attach(|py| {
            assert!(
                error.is_instance_of::<crate::errors::BacnetProtocolError>(py),
                "{value}: {error}"
            );
        });
    }
    let object = analog_input(1, "AI", 21.5).build(Storage::NONE).unwrap();
    assert_eq!(
        object
            .read_property(bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        bacnet_types::primitives::PropertyValue::Real(21.5)
    );
}

/// `admit` checks in one order, whatever else is wrong: the identifier,
/// then uniqueness, then the checks made without building, and builds only
/// once all pass.
#[test]
fn admit_reports_the_first_fault_in_one_order() {
    Python::initialize();
    let taken = [(ObjectType::DEVICE, 9, "Host")];
    let refused = || -> PyResult<()> { Err(PyValueError::new_err("pure check")) };
    let steps = std::cell::RefCell::new(Vec::new());
    let run = |(instance, name): (u32, &str), check_passes: bool| {
        steps.borrow_mut().clear();
        admit(
            (ObjectType::ANALOG_INPUT, instance, name),
            taken,
            || {
                steps.borrow_mut().push("check");
                if check_passes { Ok(()) } else { refused() }
            },
            || {
                steps.borrow_mut().push("build");
                Ok(())
            },
        )
        .map_err(|error| refusal(error).1)
    };
    // Reserved instance, a taken name and a failing check: the instance.
    let first = run((4_194_303, "Host"), false).unwrap_err();
    assert!(first.contains("4194303"), "{first}");
    assert!(steps.borrow().is_empty());
    // A taken name and a failing check: the name.
    assert_eq!(
        run((1, "Host"), false).unwrap_err(),
        "ValueError: duplicate object name: Host"
    );
    assert!(steps.borrow().is_empty());
    // A failing check alone stops before the build.
    assert_eq!(
        run((1, "Fresh"), false).unwrap_err(),
        "ValueError: pure check"
    );
    assert_eq!(*steps.borrow(), ["check"]);
    run((1, "Fresh"), true).unwrap();
    assert_eq!(*steps.borrow(), ["check", "build"]);
}

/// A profile the object would refuse is refused by `check`, before any
/// build, with the error `set_profile` would give.
#[test]
fn a_profile_is_checked_before_the_build() {
    Python::initialize();
    let mut registration = analog_input(1, "AI", 1.0);
    registration.check().unwrap();
    registration.profile = Some(ObjectProfile {
        profile_name: Some("room".into()),
        ..ObjectProfile::default()
    });
    let error = registration.check().unwrap_err();
    Python::attach(|py| {
        assert!(
            error.is_instance_of::<crate::errors::BacnetProtocolError>(py),
            "{error}"
        );
    });
}
