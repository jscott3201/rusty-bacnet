use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{
    lighting::{BinaryLightingOutputObject, LightingOutputObject},
    traits::BACnetObject,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

fn lighting_objects(configured: bool) -> [Box<dyn BACnetObject>; 2] {
    let mut lo = LightingOutputObject::new(7, "LO-7").unwrap();
    let mut blo = BinaryLightingOutputObject::new(7, "BLO-7").unwrap();
    if configured {
        lo.write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
        lo.write_property_from(
            P::LIGHTING_COMMAND,
            None,
            PropertyValue::ApplicationData(vec![0x09, 0x0A]),
            None,
            &crate::command_source::test_origin(),
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
        blo.write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
            &crate::command_source::test_origin(),
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
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::CURRENT_COMMAND_PRIORITY,
            P::COV_INCREMENT,
            P::VALUE_SOURCE,
            P::VALUE_SOURCE_ARRAY,
            P::LAST_COMMAND_TIME,
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
            P::CURRENT_COMMAND_PRIORITY,
            P::VALUE_SOURCE,
            P::VALUE_SOURCE_ARRAY,
            P::LAST_COMMAND_TIME,
        ],
    };
    // Default_Fade_Time, Default_Ramp_Rate and Default_Step_Increment follow
    // Relinquish_Default on Lighting Output only; the value-source rows
    // (#1866) end both projections.
    // COV_Increment is the last optional row on Lighting Output.
    let mut optional = vec![P::DESCRIPTION, P::RELIABILITY];
    if kind == ObjectType::LIGHTING_OUTPUT {
        optional.push(P::COV_INCREMENT);
    }
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
            assert!(
                matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::OBJECT
                    && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE)
            );
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
