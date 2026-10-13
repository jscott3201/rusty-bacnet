use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::ErrorDetail;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn py(object_type: ObjectType, instance: u32) -> PyObjectIdentifier {
    PyObjectIdentifier::from_rust(oid(object_type, instance))
}

fn size(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, Some(0)).unwrap()
}

fn is_value_out_of_range(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::PROPERTY.to_raw() as u32
            && *code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32)
}

/// The references Python gives, converted as the registration methods do.
fn references(
    references: Vec<PyDeviceObjectReference>,
) -> Option<Vec<BACnetDeviceObjectReference>> {
    device_references(Some(references), "door_members").unwrap()
}

#[test]
fn python_door_members_and_access_doors_reach_the_arrays() {
    let local = || PyDeviceObjectReference::Local(py(ObjectType::BINARY_INPUT, 3));
    let remote = || {
        PyDeviceObjectReference::Remote(py(ObjectType::DEVICE, 99), py(ObjectType::ACCESS_DOOR, 4))
    };
    let members = |references| DoorSettings {
        door_members: references,
        ..DoorSettings::default()
    };
    let door = access_door(1, "DOOR-1", members(references(vec![local(), remote()]))).unwrap();
    let mut expected = AccessDoorObject::new(1, "DOOR-1").unwrap();
    expected
        .set_door_members([
            BACnetDeviceObjectReference::from(oid(ObjectType::BINARY_INPUT, 3)),
            BACnetDeviceObjectReference {
                device_identifier: Some(oid(ObjectType::DEVICE, 99)),
                object_identifier: oid(ObjectType::ACCESS_DOOR, 4),
            },
        ])
        .unwrap();
    for index in [Some(0), None] {
        assert_eq!(
            door.read_property(PropertyIdentifier::DOOR_MEMBERS, index)
                .unwrap(),
            expected
                .read_property(PropertyIdentifier::DOOR_MEMBERS, index)
                .unwrap()
        );
    }

    let doors = |references| PointSettings {
        access_doors: references,
        ..PointSettings::default()
    };
    let point = access_point(1, "AP-1", doors(references(vec![remote()]))).unwrap();
    assert_eq!(
        size(&point, PropertyIdentifier::ACCESS_DOORS),
        PropertyValue::Unsigned(1)
    );
    // Access_Doors names Access Doors only.
    let refused = access_point(2, "AP-2", doors(references(vec![local()])))
        .err()
        .unwrap();
    assert!(is_value_out_of_range(&refused), "{refused:?}");

    // Omitted arguments keep the empty arrays.
    let door = access_door(3, "DOOR-3", DoorSettings::default()).unwrap();
    assert_eq!(
        size(&door, PropertyIdentifier::DOOR_MEMBERS),
        PropertyValue::Unsigned(0)
    );
    let point = access_point(3, "AP-3", PointSettings::default()).unwrap();
    assert_eq!(
        size(&point, PropertyIdentifier::ACCESS_DOORS),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn python_entry_and_exit_points_reach_the_zone_lists() {
    let here = || PyDeviceObjectReference::Local(py(ObjectType::ACCESS_POINT, 1));
    let remote = || {
        PyDeviceObjectReference::Remote(py(ObjectType::DEVICE, 99), py(ObjectType::ACCESS_POINT, 4))
    };
    let settings = ZoneSettings {
        entry_points: references(vec![here(), remote()]),
        exit_points: references(vec![remote()]),
        ..ZoneSettings::default()
    };
    let zone = access_zone(1, "ZONE-1", settings).unwrap();
    let remote_point = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 99)),
        object_identifier: oid(ObjectType::ACCESS_POINT, 4),
    };
    let mut expected = AccessZoneObject::new(1, "ZONE-1").unwrap();
    expected
        .set_entry_points([
            oid(ObjectType::ACCESS_POINT, 1).into(),
            remote_point.clone(),
        ])
        .unwrap();
    expected.set_exit_points([remote_point]).unwrap();
    for property in [
        PropertyIdentifier::ENTRY_POINTS,
        PropertyIdentifier::EXIT_POINTS,
    ] {
        assert_eq!(
            zone.read_property(property, None).unwrap(),
            expected.read_property(property, None).unwrap(),
            "{property:?}"
        );
    }
    // Both lists name Access Points only (#1306).
    let door = || PyDeviceObjectReference::Local(py(ObjectType::ACCESS_DOOR, 1));
    for (entry, exit) in [(vec![door()], vec![]), (vec![], vec![here(), door()])] {
        let settings = ZoneSettings {
            entry_points: references(entry),
            exit_points: references(exit),
            ..ZoneSettings::default()
        };
        let refused = access_zone(2, "ZONE-2", settings).err().unwrap();
        assert!(is_value_out_of_range(&refused), "{refused:?}");
    }
    // Omitted arguments keep the lists empty.
    let zone = access_zone(3, "ZONE-3", ZoneSettings::default()).unwrap();
    for property in [
        PropertyIdentifier::ENTRY_POINTS,
        PropertyIdentifier::EXIT_POINTS,
    ] {
        assert_eq!(
            zone.read_property(property, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }
}

#[test]
fn python_user_references_reach_the_user_lists() {
    let here = |object_type| PyDeviceObjectReference::Local(py(object_type, 1));
    let remote = |object_type| {
        PyDeviceObjectReference::Remote(py(ObjectType::DEVICE, 99), py(object_type, 4))
    };
    let in_device_99 = |object_type| BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 99)),
        object_identifier: oid(object_type, 4),
    };
    let settings = UserSettings {
        credentials: references(vec![
            here(ObjectType::ACCESS_CREDENTIAL),
            remote(ObjectType::ACCESS_CREDENTIAL),
        ]),
        members: references(vec![here(ObjectType::ACCESS_USER)]),
        member_of: references(vec![remote(ObjectType::ACCESS_USER)]),
    };
    let user = access_user(1, "USER-1", settings).unwrap();
    let mut expected = AccessUserObject::new(1, "USER-1").unwrap();
    expected
        .set_credentials([
            oid(ObjectType::ACCESS_CREDENTIAL, 1).into(),
            in_device_99(ObjectType::ACCESS_CREDENTIAL),
        ])
        .unwrap();
    expected
        .set_members([oid(ObjectType::ACCESS_USER, 1)])
        .unwrap();
    expected
        .set_member_of([in_device_99(ObjectType::ACCESS_USER)])
        .unwrap();
    let lists = [
        PropertyIdentifier::CREDENTIALS,
        PropertyIdentifier::MEMBERS,
        PropertyIdentifier::MEMBER_OF,
    ];
    for property in lists {
        assert_eq!(
            user.read_property(property, None).unwrap(),
            expected.read_property(property, None).unwrap(),
            "{property:?}"
        );
    }
    // Credentials names Access Credentials only, and the other two Access
    // Users only.
    for settings in [
        UserSettings {
            credentials: references(vec![here(ObjectType::ACCESS_USER)]),
            ..UserSettings::default()
        },
        UserSettings {
            members: references(vec![remote(ObjectType::ACCESS_CREDENTIAL)]),
            ..UserSettings::default()
        },
        UserSettings {
            member_of: references(vec![here(ObjectType::ACCESS_ZONE)]),
            ..UserSettings::default()
        },
    ] {
        let refused = access_user(2, "USER-2", settings).err().unwrap();
        assert!(is_value_out_of_range(&refused), "{refused:?}");
    }
    // Omitted arguments keep the lists empty.
    let user = access_user(3, "USER-3", UserSettings::default()).unwrap();
    for property in lists {
        assert_eq!(
            user.read_property(property, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }
}

#[test]
fn python_device_reference_pairs_name_a_device() {
    Python::initialize();
    let door = py(ObjectType::ACCESS_DOOR, 4);
    // A pair's device must be a Device (#1285): anything else raises
    // ValueError, wherever it sits in the list.
    for not_a_device in [
        py(ObjectType::ANALOG_VALUE, 99),
        py(ObjectType::ACCESS_DOOR, 99),
    ] {
        let error = device_references(
            Some(vec![
                PyDeviceObjectReference::Local(door.clone()),
                PyDeviceObjectReference::Remote(not_a_device, door.clone()),
            ]),
            "door_members",
        )
        .err()
        .unwrap();
        Python::attach(|py| {
            assert!(error.is_instance_of::<PyValueError>(py), "{error}");
            assert_eq!(
                error.value(py).to_string(),
                "door_members[1]: the device must be a Device object identifier"
            );
        });
    }
    // A Device pair and a bare identifier still convert, and no list at all
    // stays none.
    let converted = references(vec![
        PyDeviceObjectReference::Remote(py(ObjectType::DEVICE, 99), door.clone()),
        PyDeviceObjectReference::Local(door),
    ])
    .unwrap();
    assert_eq!(converted.len(), 2);
    assert!(device_references(None, "door_members").unwrap().is_none());
}

#[test]
fn python_supported_formats_reach_both_arrays() {
    // Wiegand 26 (8) in class 0, and vendor 260's format 7 (CUSTOM, 2) in
    // class 3.
    let reader = credential_data_input(
        1,
        "CDI-1",
        Some(vec![
            (PyFactorFormat::Standard(8), 0),
            (PyFactorFormat::Vendor(2, Some(260), Some(7)), 3),
        ]),
    )
    .unwrap();
    // format type [0] CUSTOM, vendor id [1] 260, vendor format [2] 7.
    assert_eq!(
        reader
            .read_property(PropertyIdentifier::SUPPORTED_FORMATS, Some(2))
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x09, 0x02, 0x1A, 0x01, 0x04, 0x29, 0x07])
    );
    assert_eq!(
        reader
            .read_property(PropertyIdentifier::SUPPORTED_FORMAT_CLASSES, None)
            .unwrap(),
        PropertyValue::List(vec![PropertyValue::Unsigned(0), PropertyValue::Unsigned(3)])
    );

    // A vendor member left out (None, as a read gives it) is absent: a
    // standard format may carry a zero vendor id alone (#1310).
    let partial = credential_data_input(
        4,
        "CDI-4",
        Some(vec![(PyFactorFormat::Vendor(10, Some(0), None), 1)]),
    )
    .unwrap();
    // format type [0] 10, vendor id [1] 0, no vendor format.
    assert_eq!(
        partial
            .read_property(PropertyIdentifier::SUPPORTED_FORMATS, Some(1))
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x09, 0x0A, 0x19, 0x00])
    );

    for formats in [
        // A CUSTOM format without its vendor members, or with one only, a
        // vendor member on another format, a type past the closed
        // production. A vendor member past Unsigned16 is OverflowError
        // before the object sees it (#1360).
        vec![(PyFactorFormat::Standard(2), 0)],
        vec![(PyFactorFormat::Vendor(2, Some(260), None), 0)],
        vec![(PyFactorFormat::Vendor(3, Some(4), None), 0)],
        vec![(PyFactorFormat::Vendor(8, Some(260), Some(7)), 0)],
        vec![(PyFactorFormat::Standard(25), 0)],
    ] {
        let refused = credential_data_input(2, "CDI-2", Some(formats))
            .err()
            .unwrap();
        assert!(is_value_out_of_range(&refused), "{refused:?}");
    }
    let bare = credential_data_input(3, "CDI-3", None).unwrap();
    assert_eq!(
        size(&bare, PropertyIdentifier::SUPPORTED_FORMATS),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn python_point_settings_reach_the_access_point_rows() {
    let read = |point: &AccessPointObject, property| point.read_property(property, None).unwrap();
    let mut point = access_point(
        1,
        "AP-1",
        PointSettings {
            number_of_authentication_policies: Some(3),
            // AUTHORIZE (0), DENY_ALL (2) and a proprietary 300.
            supported_authorization_modes: Some(vec![0, 2, 300]),
            priority_for_writing: Some(8),
            ..PointSettings::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(
            &point,
            PropertyIdentifier::NUMBER_OF_AUTHENTICATION_POLICIES
        ),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        read(&point, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(8)
    );
    // The supported modes gate Authorization_Mode writes.
    for (mode, accepted) in [(300, true), (1, false), (2, true)] {
        let result = point.write_property(
            PropertyIdentifier::AUTHORIZATION_MODE,
            None,
            PropertyValue::Enumerated(mode),
            None,
        );
        assert_eq!(result.is_ok(), accepted, "{mode}: {result:?}");
    }

    // Omitted arguments keep the defaults: one policy, priority 16 and
    // AUTHORIZE as the only supported mode, so DENY_ALL (2) is refused.
    let mut bare = access_point(2, "AP-2", PointSettings::default()).unwrap();
    assert_eq!(
        read(&bare, PropertyIdentifier::NUMBER_OF_AUTHENTICATION_POLICIES),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        read(&bare, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(16)
    );
    let refused = bare
        .write_property(
            PropertyIdentifier::AUTHORIZATION_MODE,
            None,
            PropertyValue::Enumerated(2),
            None,
        )
        .unwrap_err();
    assert!(is_value_out_of_range(&refused), "{refused:?}");

    // Zero policies, a set without AUTHORIZE, a reserved mode, and
    // priorities outside 1..=16, one of them too wide for u8.
    let refusals = [
        PointSettings {
            number_of_authentication_policies: Some(0),
            ..PointSettings::default()
        },
        PointSettings {
            supported_authorization_modes: Some(vec![1, 2]),
            ..PointSettings::default()
        },
        PointSettings {
            supported_authorization_modes: Some(vec![0, 6]),
            ..PointSettings::default()
        },
        PointSettings {
            priority_for_writing: Some(0),
            ..PointSettings::default()
        },
        PointSettings {
            priority_for_writing: Some(17),
            ..PointSettings::default()
        },
    ];
    for settings in refusals {
        let refused = access_point(3, "AP-3", settings).err().unwrap();
        assert!(is_value_out_of_range(&refused), "{refused:?}");
    }
}

#[test]
fn python_authentication_policies_reach_both_arrays_before_the_count() {
    use bacnet_types::constructed::{BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry};
    let card = BACnetAuthenticationPolicy {
        policy: vec![BACnetAuthenticationPolicyEntry {
            credential_data_input: oid(ObjectType::CREDENTIAL_DATA_INPUT, 1).into(),
            index: 1,
        }],
        order_enforced: false,
        timeout: 0,
    };
    let point = |policies: Vec<(String, BACnetAuthenticationPolicy)>, count| {
        access_point(
            1,
            "AP-1",
            PointSettings {
                authentication_policies: Some(policies),
                number_of_authentication_policies: count,
                ..PointSettings::default()
            },
        )
    };
    // The pairs set both arrays and the count.
    let set = point(vec![("card".into(), card.clone())], None).unwrap();
    for property in [
        PropertyIdentifier::AUTHENTICATION_POLICY_LIST,
        PropertyIdentifier::AUTHENTICATION_POLICY_NAMES,
    ] {
        assert_eq!(size(&set, property), PropertyValue::Unsigned(1));
    }
    assert_eq!(
        set.read_property(PropertyIdentifier::AUTHENTICATION_POLICY_NAMES, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("card".into())
    );
    // A count given as well applies after them, so it resizes both.
    let grown = point(vec![("card".into(), card)], Some(3)).unwrap();
    for property in [
        PropertyIdentifier::AUTHENTICATION_POLICY_LIST,
        PropertyIdentifier::AUTHENTICATION_POLICY_NAMES,
        PropertyIdentifier::NUMBER_OF_AUTHENTICATION_POLICIES,
    ] {
        let value = if property == PropertyIdentifier::NUMBER_OF_AUTHENTICATION_POLICIES {
            grown.read_property(property, None).unwrap()
        } else {
            size(&grown, property)
        };
        assert_eq!(value, PropertyValue::Unsigned(3), "{property:?}");
    }
    // No pairs at all is refused.
    let refused = point(Vec::new(), None).err().unwrap();
    assert!(is_value_out_of_range(&refused), "{refused:?}");
}

#[test]
fn python_door_alarm_lists_reach_the_door() {
    let settings = DoorSettings {
        alarm_values: Some(vec![2, 3]),
        fault_values: Some(vec![5, 256]),
        masked_alarm_values: Some(vec![4]),
        ..DoorSettings::default()
    };
    let door = access_door(1, "DOOR-1", settings).unwrap();
    let enumerated = |raw: &[u32]| {
        PropertyValue::List(raw.iter().copied().map(PropertyValue::Enumerated).collect())
    };
    for (property, raw) in [
        (PropertyIdentifier::ALARM_VALUES, &[2, 3][..]),
        (PropertyIdentifier::FAULT_VALUES, &[5, 256][..]),
        (PropertyIdentifier::MASKED_ALARM_VALUES, &[4][..]),
    ] {
        assert_eq!(
            door.read_property(property, None).unwrap(),
            enumerated(raw),
            "{property:?}"
        );
    }

    // A reserved state, or NORMAL in any list (#1149).
    let out_of_range = |error: &Error| {
        matches!(error, Error::Structured { class, code, .. }
            if *class == ErrorClass::PROPERTY.to_raw() as u32
                && *code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32)
    };
    for settings in [
        DoorSettings {
            alarm_values: Some(vec![9]),
            ..DoorSettings::default()
        },
        DoorSettings {
            fault_values: Some(vec![65_536]),
            ..DoorSettings::default()
        },
        DoorSettings {
            alarm_values: Some(vec![0]),
            ..DoorSettings::default()
        },
        DoorSettings {
            fault_values: Some(vec![5, 0]),
            ..DoorSettings::default()
        },
        DoorSettings {
            masked_alarm_values: Some(vec![0]),
            ..DoorSettings::default()
        },
    ] {
        let refused = access_door(2, "DOOR-2", settings).err().unwrap();
        assert!(out_of_range(&refused), "{refused:?}");
    }

    // Omitted arguments keep the lists empty.
    let door = access_door(3, "DOOR-3", DoorSettings::default()).unwrap();
    for property in [
        PropertyIdentifier::ALARM_VALUES,
        PropertyIdentifier::FAULT_VALUES,
        PropertyIdentifier::MASKED_ALARM_VALUES,
    ] {
        assert_eq!(
            door.read_property(property, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }
}

#[test]
fn python_zone_alarm_values_reach_the_zone() {
    // ABOVE_UPPER_LIMIT (4), NOT_SUPPORTED (6) and a proprietary 64.
    let settings = ZoneSettings {
        alarm_values: Some(vec![4, 6, 64]),
        ..ZoneSettings::default()
    };
    let zone = access_zone(1, "ZONE-1", settings).unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::ALARM_VALUES, None)
            .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::Enumerated(4),
            PropertyValue::Enumerated(6),
            PropertyValue::Enumerated(64),
        ])
    );

    // NORMAL (#1401), a reserved state and one past 65535: each refusal
    // names the element, counting from 1.
    for (values, position) in [
        (vec![4, 0], 2),
        (vec![0], 1),
        (vec![7], 1),
        (vec![4, 5, 65_536], 3),
    ] {
        let settings = ZoneSettings {
            alarm_values: Some(values.clone()),
            ..ZoneSettings::default()
        };
        let refused = access_zone(2, "ZONE-2", settings).err().unwrap();
        assert!(
            matches!(&refused, Error::Structured { class, code, detail }
                if *class == ErrorClass::PROPERTY.to_raw() as u32
                    && *code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32
                    && **detail == ErrorDetail::FirstFailedElementNumber(position)),
            "{values:?}: {refused:?}"
        );
    }

    // Omitted, the list starts empty.
    let zone = access_zone(3, "ZONE-3", ZoneSettings::default()).unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::ALARM_VALUES, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}
