use super::*;
use bacnet_objects::{
    life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn point_object(configured: bool, out_of_service: bool) -> LifeSafetyPointObject {
    let mut object = LifeSafetyPointObject::new(7, "LSP-7").unwrap();
    if configured {
        object.set_description("long life safety label".repeat(100));
        object.set_direct_reading(42.5);
        object.add_member(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 9).unwrap());
    }
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
    object
}

fn zone_object(configured: bool, out_of_service: bool) -> LifeSafetyZoneObject {
    let mut object = LifeSafetyZoneObject::new(7, "LSZ-7").unwrap();
    if configured {
        object.set_description("long life safety label".repeat(100));
        object.add_zone_member(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap());
    }
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
    object
}

#[test]
fn rpm_life_safety_point_metadata_selectors_preserve_bytes_and_budgets() {
    // Independent fixtures preserve the legacy projection order; the REQUIRED
    // set replaces the universal four with the Clause 12.15 R/W rows.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::DESCRIPTION,
        P::PRESENT_VALUE,
        P::MODE,
        P::SILENCED,
        P::OPERATION_EXPECTED,
        P::TRACKING_VALUE,
        P::MEMBER_OF,
        P::DIRECT_READING,
        P::MAINTENANCE_REQUIRED,
        P::EVENT_STATE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::MODE,
        P::SILENCED,
        P::OPERATION_EXPECTED,
        P::TRACKING_VALUE,
        P::EVENT_STATE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let optional = [
        P::DESCRIPTION,
        P::MEMBER_OF,
        P::DIRECT_READING,
        P::MAINTENANCE_REQUIRED,
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let object = point_object(configured, out_of_service);
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
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
fn rpm_life_safety_zone_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::DESCRIPTION,
        P::PRESENT_VALUE,
        P::MODE,
        P::SILENCED,
        P::OPERATION_EXPECTED,
        P::ZONE_MEMBERS,
        P::EVENT_STATE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::MODE,
        P::SILENCED,
        P::OPERATION_EXPECTED,
        P::ZONE_MEMBERS,
        P::EVENT_STATE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let optional = [P::DESCRIPTION];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let object = zone_object(configured, out_of_service);
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
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
fn rpm_life_safety_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    for object_type in [ObjectType::LIFE_SAFETY_POINT, ObjectType::LIFE_SAFETY_ZONE] {
        let oid = ObjectIdentifier::new(object_type, 7).unwrap();
        for object_specifier in [
            ObjectSpecifier::Type(object_type),
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
fn life_safety_delete_object_removes_point_and_zone() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(LifeSafetyPointObject::new(7, "LSP-7").unwrap()))
        .unwrap();
    db.add(Box::new(LifeSafetyZoneObject::new(7, "LSZ-7").unwrap()))
        .unwrap();
    for object_type in [ObjectType::LIFE_SAFETY_POINT, ObjectType::LIFE_SAFETY_ZONE] {
        let oid = ObjectIdentifier::new(object_type, 7).unwrap();
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid,
        }
        .encode(&mut request);
        handle_delete_object(&mut db, &request).unwrap();
        assert!(db.get(&oid).is_none());
    }
}
