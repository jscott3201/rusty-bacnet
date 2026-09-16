use super::*;
use bacnet_objects::{
    access_control::{AccessDoorObject, AccessPointObject, AccessZoneObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn access_objects(configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut door = AccessDoorObject::new(7, "DOOR-7").unwrap();
    let mut point = AccessPointObject::new(7, "AP-7").unwrap();
    let mut zone = AccessZoneObject::new(7, "ZONE-7").unwrap();
    if configured {
        door.write_property(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        door.write_property(
            P::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        point
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(2), None)
            .unwrap();
        zone.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
            .unwrap();
        zone.write_property(
            P::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(99),
            None,
        )
        .unwrap();
    }
    let mut objects: [Box<dyn BACnetObject>; 3] = [Box::new(door), Box::new(point), Box::new(zone)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long access label".repeat(100)),
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
        ObjectType::ACCESS_DOOR => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::DOOR_STATUS,
            P::LOCK_STATUS,
            P::SECURED_STATUS,
            P::DOOR_ALARM_STATE,
            P::DOOR_MEMBERS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
        ],
        ObjectType::ACCESS_POINT => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::ACCESS_EVENT,
            P::ACCESS_EVENT_TAG,
            P::ACCESS_EVENT_TIME,
            P::ACCESS_DOORS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::GLOBAL_IDENTIFIER,
            P::OCCUPANCY_COUNT,
            P::ACCESS_DOORS,
            P::ENTRY_POINTS,
            P::EXIT_POINTS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
    };
    let optional = match kind {
        ObjectType::ACCESS_DOOR => vec![
            P::DESCRIPTION,
            P::DOOR_STATUS,
            P::LOCK_STATUS,
            P::SECURED_STATUS,
            P::DOOR_ALARM_STATE,
            P::DOOR_MEMBERS,
        ],
        ObjectType::ACCESS_POINT => vec![P::DESCRIPTION, P::PRESENT_VALUE],
        _ => vec![
            P::DESCRIPTION,
            P::PRESENT_VALUE,
            P::OCCUPANCY_COUNT,
            P::ACCESS_DOORS,
        ],
    };
    let required: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !optional.contains(p))
        .collect();
    (all, required, optional)
}

#[test]
fn rpm_access_topology_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in access_objects(configured) {
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
fn rpm_access_topology_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::ACCESS_DOOR,
            ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_POINT,
            ObjectIdentifier::new(ObjectType::ACCESS_POINT, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_ZONE,
            ObjectIdentifier::new(ObjectType::ACCESS_ZONE, 7).unwrap(),
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
fn access_topology_delete_object_removes_each_trio_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in access_objects(false) {
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
