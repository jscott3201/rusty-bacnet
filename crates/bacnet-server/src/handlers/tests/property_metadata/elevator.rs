use super::*;
use bacnet_objects::{
    elevator::{ElevatorGroupObject, EscalatorObject, LiftObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn elevator_objects(configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut group = ElevatorGroupObject::new(7, "EG-7").unwrap();
    let mut escalator = EscalatorObject::new(7, "ESC-7").unwrap();
    let mut lift = LiftObject::new(7, "LIFT-7", 2).unwrap();
    if configured {
        let lift1 = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
        let lift2 = ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap();
        group.add_member(lift1);
        group.add_member(lift2);
        escalator
            .write_property(P::POWER_MODE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        escalator
            .write_property(
                P::OPERATION_DIRECTION,
                None,
                PropertyValue::Enumerated(2),
                None,
            )
            .unwrap();
        escalator
            .write_property(P::ENERGY_METER, None, PropertyValue::Real(18.75), None)
            .unwrap();
        lift.write_property(P::TRACKING_VALUE, None, PropertyValue::Unsigned(2), None)
            .unwrap();
    }
    let mut objects: [Box<dyn BACnetObject>; 3] =
        [Box::new(group), Box::new(escalator), Box::new(lift)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long elevator label".repeat(100)),
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
        ObjectType::ELEVATOR_GROUP => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::GROUP_ID,
            P::GROUP_MEMBERS,
            P::GROUP_MODE,
            P::LANDING_CALLS,
            P::LANDING_CALL_CONTROL,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        ObjectType::ESCALATOR => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::ESCALATOR_MODE,
            P::FAULT_SIGNALS,
            P::ENERGY_METER,
            P::ENERGY_METER_REF,
            P::POWER_MODE,
            P::OPERATION_DIRECTION,
            P::PASSENGER_ALARM,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ],
        _ => vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::TRACKING_VALUE,
            P::CAR_POSITION,
            P::CAR_MOVING_DIRECTION,
            P::CAR_DOOR_STATUS,
            P::CAR_LOAD,
            P::LANDING_DOOR_STATUS,
            P::FLOOR_TEXT,
            P::ENERGY_METER,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::FLOOR_NUMBER,
        ],
    };
    let optional = match kind {
        ObjectType::ELEVATOR_GROUP => vec![
            P::DESCRIPTION,
            P::GROUP_MODE,
            P::LANDING_CALLS,
            P::LANDING_CALL_CONTROL,
        ],
        ObjectType::ESCALATOR => vec![
            P::DESCRIPTION,
            P::ESCALATOR_MODE,
            P::FAULT_SIGNALS,
            P::ENERGY_METER,
            P::ENERGY_METER_REF,
            P::POWER_MODE,
            P::RELIABILITY,
        ],
        _ => vec![
            P::DESCRIPTION,
            P::TRACKING_VALUE,
            P::CAR_LOAD,
            P::LANDING_DOOR_STATUS,
            P::FLOOR_TEXT,
            P::ENERGY_METER,
            P::RELIABILITY,
            P::FLOOR_NUMBER,
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
fn rpm_elevator_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in elevator_objects(configured) {
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
fn rpm_elevator_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::ELEVATOR_GROUP,
            ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, 7).unwrap(),
        ),
        (
            ObjectType::ESCALATOR,
            ObjectIdentifier::new(ObjectType::ESCALATOR, 7).unwrap(),
        ),
        (
            ObjectType::LIFT,
            ObjectIdentifier::new(ObjectType::LIFT, 7).unwrap(),
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
fn elevator_delete_object_removes_each_trio_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in elevator_objects(false) {
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
