//! Accepted_Modes on Life Safety Point and Zone (Clauses 12.15.13 and
//! 12.16.13, #1092): the list a network write of Mode is checked against.

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, LifeSafetyMode};
use bacnet_types::error::RemoteError;

fn modes(object: &dyn BACnetObject) -> Vec<u32> {
    match object
        .read_property(PropertyIdentifier::ACCEPTED_MODES, None)
        .unwrap()
    {
        PropertyValue::List(values) => values
            .into_iter()
            .map(|value| match value {
                PropertyValue::Enumerated(raw) => raw,
                other => panic!("expected an enumerated mode, got {other:?}"),
            })
            .collect(),
        other => panic!("expected a list, got {other:?}"),
    }
}

fn mode(object: &dyn BACnetObject) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::MODE, None)
        .unwrap()
}

fn network_write_mode(object: &mut dyn BACnetObject, raw: u32) -> Result<(), Error> {
    object.write_property(
        PropertyIdentifier::MODE,
        None,
        PropertyValue::Enumerated(raw),
        None,
    )
}

fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: actual }))
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// A fresh Point and a fresh Zone, each restricted to `accepted`.
fn objects(accepted: &[LifeSafetyMode]) -> [Box<dyn BACnetObject>; 2] {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_accepted_modes(accepted.iter().copied());
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_accepted_modes(accepted.iter().copied());
    [Box::new(point), Box::new(zone)]
}

#[test]
fn accepted_modes_default_to_every_standard_mode_as_a_list() {
    let standard: Vec<u32> = (0..=19).collect();
    let point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    for object in [&point as &dyn BACnetObject, &zone] {
        assert_eq!(modes(object), standard);
        assert!(object.is_list_property(PropertyIdentifier::ACCEPTED_MODES));
        assert!(!object.is_array_property(PropertyIdentifier::ACCEPTED_MODES));
        assert!(
            object
                .property_list()
                .contains(&PropertyIdentifier::ACCEPTED_MODES)
        );
        assert!(
            object
                .required_properties()
                .contains(&PropertyIdentifier::ACCEPTED_MODES)
        );
    }
}

#[test]
fn mode_write_outside_accepted_modes_is_value_out_of_range_and_keeps_mode() {
    let accepted = [
        LifeSafetyMode::OFF,
        LifeSafetyMode::ON,
        LifeSafetyMode::TEST,
    ];
    for mut object in objects(&accepted) {
        network_write_mode(object.as_mut(), LifeSafetyMode::TEST.to_raw()).unwrap();
        assert_eq!(
            mode(object.as_ref()),
            PropertyValue::Enumerated(LifeSafetyMode::TEST.to_raw())
        );
        // A standard mode left off the list, a reserved one and a
        // proprietary one are all refused the same way.
        for refused in [LifeSafetyMode::ARMED.to_raw(), 20, 300, u32::MAX] {
            assert_property_error(
                network_write_mode(object.as_mut(), refused),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                mode(object.as_ref()),
                PropertyValue::Enumerated(LifeSafetyMode::TEST.to_raw()),
                "a refused Mode write leaves Mode alone"
            );
        }
        // The datatype check still comes first.
        assert_property_error(
            object.write_property(
                PropertyIdentifier::MODE,
                None,
                PropertyValue::Unsigned(1),
                None,
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(modes(object.as_ref()), [0, 1, 2]);
    }
}

#[test]
fn a_listed_proprietary_mode_is_accepted_and_reads_back_verbatim() {
    let proprietary = LifeSafetyMode::from_raw(300);
    for mut object in objects(&[LifeSafetyMode::OFF, proprietary]) {
        network_write_mode(object.as_mut(), 300).unwrap();
        assert_eq!(mode(object.as_ref()), PropertyValue::Enumerated(300));
        network_write_mode(object.as_mut(), LifeSafetyMode::OFF.to_raw()).unwrap();
        assert_eq!(mode(object.as_ref()), PropertyValue::Enumerated(0));
    }
}

#[test]
fn an_empty_accepted_modes_refuses_every_mode_write() {
    for mut object in objects(&[]) {
        assert!(modes(object.as_ref()).is_empty());
        assert_property_error(
            network_write_mode(object.as_mut(), LifeSafetyMode::OFF.to_raw()),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
}

#[test]
fn set_accepted_modes_keeps_each_mode_once_in_the_given_order() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_accepted_modes([
        LifeSafetyMode::ON,
        LifeSafetyMode::ARMED,
        LifeSafetyMode::ON,
        LifeSafetyMode::OFF,
    ]);
    assert_eq!(modes(&point), [1, 5, 0]);
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_accepted_modes([LifeSafetyMode::DISARMED, LifeSafetyMode::DISARMED]);
    assert_eq!(modes(&zone), [6]);
}

#[test]
fn local_mode_changes_ignore_accepted_modes_and_leave_the_list_alone() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_accepted_modes([LifeSafetyMode::ON]);
    point.set_mode(LifeSafetyMode::FAST);
    assert_eq!(
        mode(&point),
        PropertyValue::Enumerated(LifeSafetyMode::FAST.to_raw())
    );
    assert_eq!(modes(&point), [1]);

    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_accepted_modes([LifeSafetyMode::ON]);
    zone.set_mode(LifeSafetyMode::SLOW);
    assert_eq!(
        mode(&zone),
        PropertyValue::Enumerated(LifeSafetyMode::SLOW.to_raw())
    );
    assert_eq!(modes(&zone), [1]);
}

#[test]
fn accepted_modes_is_network_read_only() {
    for mut object in objects(&[LifeSafetyMode::OFF, LifeSafetyMode::ON]) {
        assert!(!object.is_writable_property(PropertyIdentifier::ACCEPTED_MODES));
        let before = object
            .read_property(PropertyIdentifier::ACCEPTED_MODES, None)
            .unwrap();
        assert_property_error(
            object.write_property(
                PropertyIdentifier::ACCEPTED_MODES,
                None,
                PropertyValue::List(vec![PropertyValue::Enumerated(5)]),
                None,
            ),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ACCEPTED_MODES, None)
                .unwrap(),
            before
        );
    }
}
