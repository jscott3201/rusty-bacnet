use super::*;
use bacnet_objects::{
    lighting::{BinaryLightingOutputObject, LightingOutputObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn lighting_objects(configured: bool) -> [Box<dyn BACnetObject>; 2] {
    let mut lo = LightingOutputObject::new(7, "LO-7").unwrap();
    let mut blo = BinaryLightingOutputObject::new(7, "BLO-7").unwrap();
    if configured {
        lo.write_property(P::PRESENT_VALUE, None, PropertyValue::Real(50.0), Some(8))
            .unwrap();
        lo.write_property(
            P::LIGHTING_COMMAND,
            None,
            PropertyValue::OctetString(vec![0x01, 0x02]),
            None,
        )
        .unwrap();
        lo.write_property(
            P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
            None,
            PropertyValue::Unsigned(8),
            None,
        )
        .unwrap();
        lo.write_property(P::RELINQUISH_DEFAULT, None, PropertyValue::Real(75.0), None)
            .unwrap();
        lo.write_property(
            P::BLINK_WARN_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        lo.write_property(P::EGRESS_TIME, None, PropertyValue::Unsigned(600), None)
            .unwrap();
        blo.write_property(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        blo.write_property(
            P::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        blo.write_property(
            P::BLINK_WARN_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        blo.write_property(P::EGRESS_TIME, None, PropertyValue::Unsigned(5), None)
            .unwrap();
    }
    let mut objects: [Box<dyn BACnetObject>; 2] = [Box::new(lo), Box::new(blo)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long lighting label".repeat(100)),
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
        ObjectType::LIGHTING_OUTPUT => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::LIGHTING_COMMAND,
            P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
            P::IN_PROGRESS,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::DEFAULT_FADE_TIME,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
        ],
    };
    // Default_Fade_Time shifts the Lighting Output projection by one row
    // after Relinquish_Default; the Binary projection has no such row.
    let optional = vec![P::DESCRIPTION, P::RELIABILITY];
    let required: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !optional.contains(p))
        .collect();
    (all, required, optional)
}

#[test]
fn rpm_lighting_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in lighting_objects(configured) {
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
fn rpm_lighting_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::LIGHTING_OUTPUT,
            ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 7).unwrap(),
        ),
        (
            ObjectType::BINARY_LIGHTING_OUTPUT,
            ObjectIdentifier::new(ObjectType::BINARY_LIGHTING_OUTPUT, 7).unwrap(),
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
fn lighting_delete_object_removes_each_duo_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in lighting_objects(false) {
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
