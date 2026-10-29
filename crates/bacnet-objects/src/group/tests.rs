//! Group, Global Group and Structured View object tests.

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

// -----------------------------------------------------------------------
// GroupObject tests
// -----------------------------------------------------------------------

#[test]
fn group_create() {
    let g = GroupObject::new(1, "Group-1").unwrap();
    assert_eq!(g.object_identifier().object_type(), ObjectType::GROUP);
    assert_eq!(g.object_identifier().instance_number(), 1);
    assert_eq!(g.object_name(), "Group-1");
}

#[test]
fn group_object_type() {
    let g = GroupObject::new(1, "G").unwrap();
    let val = g
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(ObjectType::GROUP.to_raw()));
}

#[test]
fn group_property_list() {
    let g = GroupObject::new(1, "G").unwrap();
    let props = g.property_list();
    assert!(props.contains(&PropertyIdentifier::LIST_OF_GROUP_MEMBERS));
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    // Table 12-17 has no Status_Flags (#1064).
    assert!(!props.contains(&PropertyIdentifier::STATUS_FLAGS));
}

// -----------------------------------------------------------------------
// GlobalGroupObject tests
// -----------------------------------------------------------------------

#[test]
fn global_group_create() {
    let gg = GlobalGroupObject::new(1, "GG-1").unwrap();
    assert_eq!(
        gg.object_identifier().object_type(),
        ObjectType::GLOBAL_GROUP
    );
    assert_eq!(gg.object_identifier().instance_number(), 1);
    assert_eq!(gg.object_name(), "GG-1");
}

#[test]
fn global_group_object_type() {
    let gg = GlobalGroupObject::new(1, "GG").unwrap();
    let val = gg
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::GLOBAL_GROUP.to_raw())
    );
}

#[test]
fn global_group_members_empty() {
    let gg = GlobalGroupObject::new(1, "GG").unwrap();
    let val = gg
        .read_property(PropertyIdentifier::GROUP_MEMBERS, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert!(items.is_empty());
    } else {
        panic!("Expected List");
    }
}

#[test]
fn global_group_member_names() {
    let mut gg = GlobalGroupObject::new(1, "GG").unwrap();
    gg.group_member_names.push("Temp Sensor".into());
    gg.group_member_names.push("Humidity".into());

    let val = gg
        .read_property(PropertyIdentifier::GROUP_MEMBER_NAMES, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            PropertyValue::CharacterString("Temp Sensor".into())
        );
        assert_eq!(items[1], PropertyValue::CharacterString("Humidity".into()));
    } else {
        panic!("Expected List");
    }
}

#[test]
fn global_group_property_list() {
    let gg = GlobalGroupObject::new(1, "GG").unwrap();
    let props = gg.property_list();
    assert!(props.contains(&PropertyIdentifier::GROUP_MEMBERS));
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::GROUP_MEMBER_NAMES));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::MEMBER_STATUS_FLAGS));
}

// -----------------------------------------------------------------------
// StructuredViewObject tests
// -----------------------------------------------------------------------

#[test]
fn structured_view_create() {
    let sv = StructuredViewObject::new(1, "SV-1").unwrap();
    assert_eq!(
        sv.object_identifier().object_type(),
        ObjectType::STRUCTURED_VIEW
    );
    assert_eq!(sv.object_identifier().instance_number(), 1);
    assert_eq!(sv.object_name(), "SV-1");
}

#[test]
fn structured_view_object_type() {
    let sv = StructuredViewObject::new(1, "SV").unwrap();
    let val = sv
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::STRUCTURED_VIEW.to_raw())
    );
}

#[test]
fn structured_view_add_subordinates() {
    let mut sv = StructuredViewObject::new(1, "SV").unwrap();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let bi1 = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    sv.add_subordinate(ai1, "Temperature").unwrap();
    sv.add_subordinate(
        BACnetDeviceObjectReference {
            device_identifier: Some(device),
            object_identifier: bi1,
        },
        "Occupancy",
    )
    .unwrap();

    // Each Subordinate_List element is a BACnetDeviceObjectReference
    // (Table 12-34): the object under [1], after the device under [0] when
    // the subordinate lives in another device.
    let subordinates = [
        PropertyValue::ApplicationData(vec![0x1C, 0x00, 0x00, 0x00, 0x01]),
        PropertyValue::ApplicationData(vec![
            0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x00, 0xC0, 0x00, 0x01,
        ]),
    ];
    let annotations = [
        PropertyValue::CharacterString("Temperature".into()),
        PropertyValue::CharacterString("Occupancy".into()),
    ];
    for (property, elements) in [
        (PropertyIdentifier::SUBORDINATE_LIST, &subordinates),
        (PropertyIdentifier::SUBORDINATE_ANNOTATIONS, &annotations),
    ] {
        assert_eq!(
            sv.read_property(property, None).unwrap(),
            PropertyValue::List(elements.to_vec()),
            "{property:?}"
        );
        // Index 0 is the size, 1..=N one element, and past N is
        // INVALID_ARRAY_INDEX.
        assert_eq!(
            sv.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(2),
            "{property:?}[0]"
        );
        for (index, element) in (1..).zip(elements) {
            assert_eq!(
                &sv.read_property(property, Some(index)).unwrap(),
                element,
                "{property:?}[{index}]"
            );
        }
        for index in [3, u32::MAX] {
            let error = sv.read_property(property, Some(index)).unwrap_err();
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::INVALID_ARRAY_INDEX),
                "{property:?}[{index}]: {error:?}"
            );
        }
        // Neither array has a write route, whole or by element.
        for index in [None, Some(0), Some(1)] {
            let error = sv
                .write_property(property, index, elements[0].clone(), None)
                .unwrap_err();
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::WRITE_ACCESS_DENIED),
                "{property:?}[{index:?}] write: {error:?}"
            );
        }
        assert_eq!(
            sv.read_property(property, None).unwrap(),
            PropertyValue::List(elements.to_vec())
        );
    }
}

#[test]
fn structured_view_add_subordinate_refuses_a_non_device_device_identifier() {
    let mut sv = StructuredViewObject::new(1, "SV").unwrap();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    sv.add_subordinate(ai1, "Temperature").unwrap();
    // A device identifier of another object type is no Device (#1285), and
    // neither array grows.
    for object_type in [ObjectType::ANALOG_VALUE, ObjectType::STRUCTURED_VIEW] {
        let result = sv.add_subordinate(
            BACnetDeviceObjectReference {
                device_identifier: Some(ObjectIdentifier::new(object_type, 9).unwrap()),
                object_identifier: ai1,
            },
            "Elsewhere",
        );
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::VALUE_OUT_OF_RANGE),
            "{object_type:?}: {result:?}"
        );
        for property in [
            PropertyIdentifier::SUBORDINATE_LIST,
            PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
        ] {
            assert_eq!(
                sv.read_property(property, Some(0)).unwrap(),
                PropertyValue::Unsigned(1),
                "{property:?}"
            );
        }
    }
}

#[test]
fn structured_view_set_subordinates_replaces_both_arrays_or_nothing() {
    let mut sv = StructuredViewObject::new(1, "SV").unwrap();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let bi2 = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 2).unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    sv.add_subordinate(ai1, "Temperature").unwrap();
    let remote = BACnetDeviceObjectReference {
        device_identifier: Some(device),
        object_identifier: bi2,
    };
    // A replace drops the old subordinates and keeps both arrays one size.
    sv.set_subordinates(vec![
        (bi2.into(), "Occupancy".to_string()),
        (remote.clone(), "Remote".to_string()),
    ])
    .unwrap();
    let served: Vec<_> = sv
        .subordinates()
        .map(|(reference, annotation)| (reference.clone(), annotation.to_string()))
        .collect();
    assert_eq!(
        served,
        [
            (bi2.into(), "Occupancy".to_string()),
            (remote, "Remote".to_string())
        ]
    );
    let size = |sv: &StructuredViewObject, property| sv.read_property(property, Some(0)).unwrap();
    for property in [
        PropertyIdentifier::SUBORDINATE_LIST,
        PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
    ] {
        assert_eq!(
            size(&sv, property),
            PropertyValue::Unsigned(2),
            "{property:?}"
        );
    }
    // One reference whose device isn't a Device refuses the whole list
    // (#1285), and the two subordinates stay.
    let not_a_device = BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap()),
        object_identifier: ai1,
    };
    let result = sv.set_subordinates(vec![
        (ai1.into(), "Temperature".to_string()),
        (not_a_device, "Elsewhere".to_string()),
    ]);
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "{result:?}"
    );
    assert_eq!(sv.subordinates().count(), 2);
    assert_eq!(sv.subordinates().next().unwrap().1, "Occupancy");
    // An empty list clears both arrays.
    sv.set_subordinates(Vec::new()).unwrap();
    for property in [
        PropertyIdentifier::SUBORDINATE_LIST,
        PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
    ] {
        assert_eq!(
            size(&sv, property),
            PropertyValue::Unsigned(0),
            "{property:?}"
        );
    }
}

#[test]
fn structured_view_node_type() {
    let sv = StructuredViewObject::new(1, "SV").unwrap();
    let val = sv
        .read_property(PropertyIdentifier::NODE_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn structured_view_node_subtype() {
    let sv = StructuredViewObject::new(1, "SV").unwrap();
    let val = sv
        .read_property(PropertyIdentifier::NODE_SUBTYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString(String::new()));
}

#[test]
fn structured_view_property_list() {
    let sv = StructuredViewObject::new(1, "SV").unwrap();
    let props = sv.property_list();
    assert!(props.contains(&PropertyIdentifier::NODE_TYPE));
    assert!(props.contains(&PropertyIdentifier::NODE_SUBTYPE));
    assert!(props.contains(&PropertyIdentifier::SUBORDINATE_LIST));
    assert!(props.contains(&PropertyIdentifier::SUBORDINATE_ANNOTATIONS));
}
