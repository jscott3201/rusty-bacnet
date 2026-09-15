use super::*;
use bacnet_objects::{
    group::{GlobalGroupObject, GroupObject, StructuredViewObject},
    traits::BACnetObject,
};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn group_objects(configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut group = GroupObject::new(7, "GRP-7").unwrap();
    let mut global = GlobalGroupObject::new(7, "GG-7").unwrap();
    let mut view = StructuredViewObject::new(7, "SV-7").unwrap();
    if configured {
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        group.add_member(ai1);
        group.add_member(ai2);
        group.present_value.push(PropertyValue::Enumerated(3));
        global
            .group_members
            .push(BACnetDeviceObjectPropertyReference {
                object_identifier: ai1,
                property_identifier: P::PRESENT_VALUE.to_raw(),
                property_array_index: None,
                device_identifier: None,
            });
        global.group_member_names.push("a".into());
        global.present_value.push(PropertyValue::Enumerated(1));
        global.present_value.push(PropertyValue::Enumerated(2));
        view.add_subordinate(ai1, "a");
        let _ = ai2;
    }
    let mut objects: [Box<dyn BACnetObject>; 3] =
        [Box::new(group), Box::new(global), Box::new(view)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long group label".repeat(100)),
                None,
            )
            .unwrap();
        // Exercise the unconditional write route so large encodings persist.
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
    }
    objects
}

fn expected_lists(kind: ObjectType) -> (Vec<P>, Vec<P>, Vec<P>) {
    let all = match kind {
        ObjectType::GROUP => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::LIST_OF_GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        ObjectType::GLOBAL_GROUP => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::GROUP_MEMBER_NAMES,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::NODE_TYPE,
            P::NODE_SUBTYPE,
            P::SUBORDINATE_LIST,
            P::SUBORDINATE_ANNOTATIONS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
    };
    let optional = match kind {
        ObjectType::GROUP => vec![P::DESCRIPTION],
        ObjectType::GLOBAL_GROUP => vec![P::DESCRIPTION, P::GROUP_MEMBER_NAMES, P::RELIABILITY],
        _ => vec![P::DESCRIPTION, P::NODE_SUBTYPE, P::SUBORDINATE_ANNOTATIONS],
    };
    let required: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !optional.contains(p))
        .collect();
    (all, required, optional)
}

#[test]
fn rpm_group_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in group_objects(configured) {
            let oid = object.object_identifier();
            let (all, required, optional) = expected_lists(oid.object_type());
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

#[test]
fn rpm_group_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::GROUP,
            ObjectIdentifier::new(ObjectType::GROUP, 7).unwrap(),
        ),
        (
            ObjectType::GLOBAL_GROUP,
            ObjectIdentifier::new(ObjectType::GLOBAL_GROUP, 7).unwrap(),
        ),
        (
            ObjectType::STRUCTURED_VIEW,
            ObjectIdentifier::new(ObjectType::STRUCTURED_VIEW, 7).unwrap(),
        ),
    ];
    for (kind, oid) in cases {
        for object_specifier in [
            ObjectSpecifier::Type(kind),
            ObjectSpecifier::Identifier(oid),
        ] {
            let mut db = ObjectDatabase::new();
            let mut request = BytesMut::new();
            CreateObjectRequest {
                object_specifier,
                list_of_initial_values: vec![],
            }
            .encode(&mut request);
            let mut response = BytesMut::new();
            let result = handle_create_object(&mut db, &request, &mut response);
            assert!(matches!(result, Err(Error::Protocol { class, code })
                if class == ErrorClass::OBJECT.to_raw() as u32
                    && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32));
            assert!(response.is_empty());
            assert!(db.is_empty());
        }
    }
}

#[test]
fn group_delete_object_removes_each_trio_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in group_objects(false) {
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid,
        }
        .encode(&mut request);
        handle_delete_object(&mut db, &request).unwrap();
        assert!(db.get(&oid).is_none());
    }
}
