use super::*;
use bacnet_objects::{
    accumulator::{AccumulatorObject, PulseConverterObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn accumulator_objects(configured: bool) -> [Box<dyn BACnetObject>; 2] {
    let mut acc = AccumulatorObject::new(7, "ACC-7", 95).unwrap();
    let mut pc = PulseConverterObject::new(7, "PC-7", 62).unwrap();
    if configured {
        acc.write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(1000), None)
            .unwrap();
        acc.write_property(P::PULSE_RATE, None, PropertyValue::Real(2.5), None)
            .unwrap();
        acc.write_property(
            P::LIMIT_MONITORING_INTERVAL,
            None,
            PropertyValue::Unsigned(60),
            None,
        )
        .unwrap();
        pc.write_property(P::SCALE_FACTOR, None, PropertyValue::Real(2.5), None)
            .unwrap();
        pc.write_property(P::ADJUST_VALUE, None, PropertyValue::Real(0.5), None)
            .unwrap();
        pc.write_property(P::COV_INCREMENT, None, PropertyValue::Real(0.5), None)
            .unwrap();
        let target = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
        pc.write_property(
            P::INPUT_REFERENCE,
            None,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(target),
                PropertyValue::Enumerated(P::PRESENT_VALUE.to_raw()),
            ]),
            None,
        )
        .unwrap();
    }
    let mut objects: [Box<dyn BACnetObject>; 2] = [Box::new(acc), Box::new(pc)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long accumulator label".repeat(100)),
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
    // The Pulse Converter gate is state-dependent: pin the out-of-service
    // Present_Value route while the object is out of service.
    if configured {
        objects[1]
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(12.5), None)
            .unwrap();
    }
    objects
}

fn expected_lists(kind: ObjectType) -> (Vec<P>, Vec<P>, Vec<P>) {
    let all = match kind {
        ObjectType::ACCUMULATOR => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::MAX_PRES_VALUE,
            P::SCALE,
            P::PRESCALE,
            P::PULSE_RATE,
            P::UNITS,
            P::LIMIT_MONITORING_INTERVAL,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::VALUE_BEFORE_CHANGE,
            P::VALUE_SET,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::UNITS,
            P::SCALE_FACTOR,
            P::ADJUST_VALUE,
            P::COV_INCREMENT,
            P::INPUT_REFERENCE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
    };
    // Table-O rows per Tables 12-79/12-27; every other served row is table R/W.
    let optional = match kind {
        ObjectType::ACCUMULATOR => vec![
            P::DESCRIPTION,
            P::PRESCALE,
            P::PULSE_RATE,
            P::LIMIT_MONITORING_INTERVAL,
            P::RELIABILITY,
            P::VALUE_BEFORE_CHANGE,
            P::VALUE_SET,
        ],
        _ => vec![
            P::DESCRIPTION,
            P::COV_INCREMENT,
            P::INPUT_REFERENCE,
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
fn rpm_accumulator_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in accumulator_objects(configured) {
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
fn rpm_accumulator_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::ACCUMULATOR,
            ObjectIdentifier::new(ObjectType::ACCUMULATOR, 7).unwrap(),
        ),
        (
            ObjectType::PULSE_CONVERTER,
            ObjectIdentifier::new(ObjectType::PULSE_CONVERTER, 7).unwrap(),
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
fn accumulator_delete_object_removes_each_pair_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in accumulator_objects(false) {
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
fn rpm_accumulator_metadata_corrects_historical_pics_writability() {
    // Intended PICS correction (call-out): the historical default advertised
    // Accumulator Object_Name and Present_Value writable while no arm routes
    // them, and hid the routed Max_Pres_Value, Pulse_Rate, and
    // Limit_Monitoring_Interval arms. The canonical rows correct both
    // directions; dispatch itself is unchanged.
    let object = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
    for p in [P::OBJECT_NAME, P::PRESENT_VALUE] {
        assert!(!object.is_writable_property(p));
    }
    for p in [
        P::MAX_PRES_VALUE,
        P::PULSE_RATE,
        P::LIMIT_MONITORING_INTERVAL,
    ] {
        assert!(object.is_writable_property(p));
    }
    // The Pulse Converter override translated exactly: the same seven
    // properties stay advertised, including gated Present_Value.
    let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
    for p in [
        P::PRESENT_VALUE,
        P::SCALE_FACTOR,
        P::ADJUST_VALUE,
        P::INPUT_REFERENCE,
        P::DESCRIPTION,
        P::OUT_OF_SERVICE,
        P::COV_INCREMENT,
    ] {
        assert!(pc.is_writable_property(p));
    }
    assert!(!pc.is_writable_property(P::OBJECT_NAME));
}
