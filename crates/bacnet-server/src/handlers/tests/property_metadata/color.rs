use super::*;
use bacnet_objects::{
    color::{ColorObject, ColorTemperatureObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn color_objects(configured: bool) -> [Box<dyn BACnetObject>; 2] {
    let mut color = ColorObject::new(7, "CLR-7").unwrap();
    let mut temperature = ColorTemperatureObject::new(7, "CT-7").unwrap();
    if configured {
        color
            .write_property(
                P::COLOR_COMMAND,
                None,
                PropertyValue::OctetString(vec![0x01, 0x02]),
                None,
            )
            .unwrap();
        color
            .write_property(
                P::DEFAULT_FADE_TIME,
                None,
                PropertyValue::Unsigned(1000),
                None,
            )
            .unwrap();
        // Present_Value has no Out_Of_Service gate: the in-range write lands
        // in both configurations.
        temperature
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(5000), None)
            .unwrap();
        temperature
            .write_property(
                P::COLOR_COMMAND,
                None,
                PropertyValue::OctetString(vec![0x04, 0x05]),
                None,
            )
            .unwrap();
    }
    let mut objects: [Box<dyn BACnetObject>; 2] = [Box::new(color), Box::new(temperature)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long color label".repeat(100)),
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
        ObjectType::COLOR => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::DEFAULT_COLOR,
            P::DEFAULT_FADE_TIME,
            P::TRANSITION,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::DEFAULT_COLOR_TEMPERATURE,
            P::DEFAULT_FADE_TIME,
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::TRANSITION,
            P::MIN_PRES_VALUE,
            P::MAX_PRES_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
    };
    // Secondary tuning rows per the dispatch-first mapping; every other
    // served row is RequiredRead.
    let optional = match kind {
        ObjectType::COLOR => vec![P::DESCRIPTION, P::DEFAULT_COLOR, P::RELIABILITY],
        _ => vec![
            P::DESCRIPTION,
            P::DEFAULT_COLOR_TEMPERATURE,
            P::DEFAULT_FADE_TIME,
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::RELIABILITY,
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
fn rpm_color_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in color_objects(configured) {
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
fn rpm_color_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::COLOR,
            ObjectIdentifier::new(ObjectType::COLOR, 7).unwrap(),
        ),
        (
            ObjectType::COLOR_TEMPERATURE,
            ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 7).unwrap(),
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
fn color_delete_object_removes_each_pair_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in color_objects(false) {
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

#[test]
fn rpm_color_metadata_corrects_historical_pics_writability() {
    // Intended PICS correction (call-out): the historical default advertised
    // Object_Name and (for Color) Present_Value writable while no arm routes
    // them, and hid the routed Color_Command and Default_Fade_Time arms. The
    // canonical rows correct both directions; dispatch itself is unchanged.
    // Color Temperature Present_Value stays advertised (its Unsigned arm is
    // routed unconditionally, with no Out_Of_Service gate).
    let color = ColorObject::new(1, "CLR-1").unwrap();
    assert!(!color.is_writable_property(P::OBJECT_NAME));
    assert!(!color.is_writable_property(P::PRESENT_VALUE));
    assert!(color.is_writable_property(P::COLOR_COMMAND));
    assert!(color.is_writable_property(P::DEFAULT_FADE_TIME));
    let temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    assert!(!temperature.is_writable_property(P::OBJECT_NAME));
    assert!(temperature.is_writable_property(P::PRESENT_VALUE));
    assert!(temperature.is_writable_property(P::COLOR_COMMAND));
    // The Default_Fade_Time asymmetry is preserved, not reconciled: only
    // Color routes the write.
    assert!(!temperature.is_writable_property(P::DEFAULT_FADE_TIME));
}
