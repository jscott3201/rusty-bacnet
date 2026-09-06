use super::super::*;
use super::{assert_invalid_data_type, assert_value_out_of_range};
use bacnet_types::enums::{EscalatorFault, EscalatorMode, EscalatorOperationDirection};

fn read(esc: &EscalatorObject, property: PropertyIdentifier) -> PropertyValue {
    esc.read_property(property, None).unwrap()
}

fn write(
    esc: &mut EscalatorObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    esc.write_property(property, None, value, None)
}

fn status_values(alternate: bool) -> Vec<(PropertyIdentifier, PropertyValue)> {
    if alternate {
        vec![
            (
                PropertyIdentifier::POWER_MODE,
                PropertyValue::Boolean(false),
            ),
            (
                PropertyIdentifier::OPERATION_DIRECTION,
                PropertyValue::Enumerated(EscalatorOperationDirection::DOWN_RATED_SPEED.to_raw()),
            ),
            (
                PropertyIdentifier::ESCALATOR_MODE,
                PropertyValue::Enumerated(EscalatorMode::DOWN.to_raw()),
            ),
            (PropertyIdentifier::ENERGY_METER, PropertyValue::Real(-4.25)),
            (
                PropertyIdentifier::FAULT_SIGNALS,
                PropertyValue::List(vec![
                    PropertyValue::Enumerated(EscalatorFault::COMB_PLATE_FAULT.to_raw()),
                    PropertyValue::Enumerated(65535),
                ]),
            ),
            (
                PropertyIdentifier::PASSENGER_ALARM,
                PropertyValue::Boolean(false),
            ),
        ]
    } else {
        vec![
            (PropertyIdentifier::POWER_MODE, PropertyValue::Boolean(true)),
            (
                PropertyIdentifier::OPERATION_DIRECTION,
                PropertyValue::Enumerated(EscalatorOperationDirection::UP_RATED_SPEED.to_raw()),
            ),
            (
                PropertyIdentifier::ESCALATOR_MODE,
                PropertyValue::Enumerated(EscalatorMode::UP.to_raw()),
            ),
            (PropertyIdentifier::ENERGY_METER, PropertyValue::Real(12.5)),
            (
                PropertyIdentifier::FAULT_SIGNALS,
                PropertyValue::List(vec![
                    PropertyValue::Enumerated(EscalatorFault::CONTROLLER_FAULT.to_raw()),
                    PropertyValue::Enumerated(1024),
                ]),
            ),
            (
                PropertyIdentifier::PASSENGER_ALARM,
                PropertyValue::Boolean(true),
            ),
        ]
    }
}

fn assert_status_values(esc: &EscalatorObject, expected: &[(PropertyIdentifier, PropertyValue)]) {
    for (property, value) in expected {
        assert_eq!(read(esc, *property), *value, "{property:?} readback");
    }
}

#[test]
fn all_six_status_properties_write_in_service_and_out_of_service() {
    for out_of_service in [false, true] {
        let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
        if out_of_service {
            write(
                &mut esc,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        }

        let values = status_values(false);
        for (property, value) in &values {
            write(&mut esc, *property, value.clone()).unwrap_or_else(|error| {
                panic!("{property:?} must write with OOS={out_of_service}: {error:?}")
            });
        }
        assert_status_values(&esc, &values);
    }
}

#[test]
fn out_of_service_toggles_preserve_client_status_values() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let in_service = status_values(false);
    for (property, value) in &in_service {
        write(&mut esc, *property, value.clone()).unwrap();
    }

    write(
        &mut esc,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_status_values(&esc, &in_service);

    let simulated = status_values(true);
    for (property, value) in &simulated {
        write(&mut esc, *property, value.clone()).unwrap();
    }
    write(
        &mut esc,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_status_values(&esc, &simulated);
}

#[test]
fn writable_predicate_matches_only_the_escalator_write_routes() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let writable = [
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::POWER_MODE,
        PropertyIdentifier::OPERATION_DIRECTION,
        PropertyIdentifier::ESCALATOR_MODE,
        PropertyIdentifier::ENERGY_METER,
        PropertyIdentifier::FAULT_SIGNALS,
        PropertyIdentifier::PASSENGER_ALARM,
    ];

    for property in esc.property_list().iter().copied() {
        assert_eq!(
            esc.is_writable_property(property),
            writable.contains(&property),
            "writability mismatch for {property:?}"
        );
    }
    for property in [
        PropertyIdentifier::PROPERTY_LIST,
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyIdentifier::OBJECT_TYPE,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::ENERGY_METER_REF,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert!(!esc.is_writable_property(property), "{property:?}");
    }
}

#[test]
fn fault_signals_accept_empty_singleton_and_typed_multi_value_sets() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(esc.fault_signals, Vec::<EscalatorFault>::new());

    write(
        &mut esc,
        PropertyIdentifier::FAULT_SIGNALS,
        PropertyValue::List(vec![]),
    )
    .unwrap();
    assert_eq!(
        read(&esc, PropertyIdentifier::FAULT_SIGNALS),
        PropertyValue::List(vec![])
    );

    for &(_, fault) in EscalatorFault::ALL_NAMED {
        let raw = fault.to_raw();
        write(
            &mut esc,
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyValue::Enumerated(raw),
        )
        .unwrap();
        assert_eq!(
            read(&esc, PropertyIdentifier::FAULT_SIGNALS),
            PropertyValue::List(vec![PropertyValue::Enumerated(raw)])
        );
    }
    for raw in [1024u32, 65535] {
        write(
            &mut esc,
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyValue::Enumerated(raw),
        )
        .unwrap();
        assert_eq!(
            read(&esc, PropertyIdentifier::FAULT_SIGNALS),
            PropertyValue::List(vec![PropertyValue::Enumerated(raw)])
        );
    }

    let set = PropertyValue::List(vec![
        PropertyValue::Enumerated(0),
        PropertyValue::Enumerated(8),
        PropertyValue::Enumerated(1024),
        PropertyValue::Enumerated(65535),
    ]);
    write(&mut esc, PropertyIdentifier::FAULT_SIGNALS, set.clone()).unwrap();
    assert_eq!(read(&esc, PropertyIdentifier::FAULT_SIGNALS), set);
}

#[test]
fn fault_signals_reject_reserved_oversized_and_duplicate_values_atomically() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let prior = PropertyValue::List(vec![
        PropertyValue::Enumerated(EscalatorFault::CONTROLLER_FAULT.to_raw()),
        PropertyValue::Enumerated(1024),
    ]);
    write(&mut esc, PropertyIdentifier::FAULT_SIGNALS, prior.clone()).unwrap();

    for raw in [9u32, 1023, 65536, u32::MAX] {
        assert_value_out_of_range(
            write(
                &mut esc,
                PropertyIdentifier::FAULT_SIGNALS,
                PropertyValue::Enumerated(raw),
            ),
            &format!("invalid fault value {raw}"),
        );
        assert_eq!(read(&esc, PropertyIdentifier::FAULT_SIGNALS), prior);
    }

    for duplicate in [0u32, 1024] {
        assert_value_out_of_range(
            write(
                &mut esc,
                PropertyIdentifier::FAULT_SIGNALS,
                PropertyValue::List(vec![
                    PropertyValue::Enumerated(duplicate),
                    PropertyValue::Enumerated(duplicate),
                ]),
            ),
            &format!("duplicate fault value {duplicate}"),
        );
        assert_eq!(read(&esc, PropertyIdentifier::FAULT_SIGNALS), prior);
    }

    let mut late_duplicate: Vec<PropertyValue> =
        (1024..=11022).map(PropertyValue::Enumerated).collect();
    late_duplicate.push(PropertyValue::Enumerated(1024));
    assert_value_out_of_range(
        write(
            &mut esc,
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyValue::List(late_duplicate),
        ),
        "late duplicate in a maximum-size service list",
    );
    assert_eq!(read(&esc, PropertyIdentifier::FAULT_SIGNALS), prior);
}

#[test]
fn fault_signals_reject_wrong_value_shapes_atomically() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let prior = PropertyValue::List(vec![PropertyValue::Enumerated(8)]);
    write(&mut esc, PropertyIdentifier::FAULT_SIGNALS, prior.clone()).unwrap();

    for wrong in [
        PropertyValue::Unsigned(8),
        PropertyValue::List(vec![
            PropertyValue::Enumerated(0),
            PropertyValue::Unsigned(8),
        ]),
    ] {
        assert_invalid_data_type(
            write(&mut esc, PropertyIdentifier::FAULT_SIGNALS, wrong),
            "mistyped Fault_Signals",
        );
        assert_eq!(read(&esc, PropertyIdentifier::FAULT_SIGNALS), prior);
    }
}

#[test]
fn non_finite_energy_meter_values_are_rejected_atomically() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    write(
        &mut esc,
        PropertyIdentifier::ENERGY_METER,
        PropertyValue::Real(42.0),
    )
    .unwrap();

    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_value_out_of_range(
            write(
                &mut esc,
                PropertyIdentifier::ENERGY_METER,
                PropertyValue::Real(value),
            ),
            "non-finite Energy_Meter",
        );
        assert_eq!(
            read(&esc, PropertyIdentifier::ENERGY_METER),
            PropertyValue::Real(42.0)
        );
    }
}

#[test]
fn all_six_status_properties_reject_wrong_datatypes_atomically() {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let cases = [
        (
            PropertyIdentifier::POWER_MODE,
            PropertyValue::Boolean(true),
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyIdentifier::OPERATION_DIRECTION,
            PropertyValue::Enumerated(EscalatorOperationDirection::UP_RATED_SPEED.to_raw()),
            PropertyValue::Unsigned(2),
        ),
        (
            PropertyIdentifier::ESCALATOR_MODE,
            PropertyValue::Enumerated(EscalatorMode::UP.to_raw()),
            PropertyValue::Unsigned(2),
        ),
        (
            PropertyIdentifier::ENERGY_METER,
            PropertyValue::Real(7.0),
            PropertyValue::Double(7.0),
        ),
        (
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyValue::List(vec![PropertyValue::Enumerated(0)]),
            PropertyValue::Unsigned(0),
        ),
        (
            PropertyIdentifier::PASSENGER_ALARM,
            PropertyValue::Boolean(true),
            PropertyValue::Enumerated(1),
        ),
    ];

    for (property, prior, wrong) in cases {
        write(&mut esc, property, prior.clone()).unwrap();
        assert_invalid_data_type(
            write(&mut esc, property, wrong),
            &format!("wrong datatype for {property:?}"),
        );
        assert_eq!(read(&esc, property), prior, "{property:?} changed");
    }
}
