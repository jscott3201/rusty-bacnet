use super::*;
use bacnet_objects::{
    elevator::{ElevatorGroupObject, EscalatorObject, LiftObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<(P, bool, bool)> {
    // Independent (identifier, optional, writable) rows in projection order.
    match kind {
        ObjectType::ELEVATOR_GROUP => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::GROUP_ID, false, true),
            (P::GROUP_MEMBERS, false, false),
            (P::GROUP_MODE, true, true),
            (P::LANDING_CALLS, true, false),
            (P::LANDING_CALL_CONTROL, true, true),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ESCALATOR => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::ESCALATOR_MODE, true, true),
            (P::FAULT_SIGNALS, true, true),
            (P::ENERGY_METER, true, true),
            (P::ENERGY_METER_REF, true, false),
            (P::POWER_MODE, true, true),
            (P::OPERATION_DIRECTION, false, true),
            (P::PASSENGER_ALARM, false, true),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::TRACKING_VALUE, true, true),
            (P::CAR_POSITION, false, true),
            (P::CAR_MOVING_DIRECTION, false, true),
            (P::CAR_DOOR_STATUS, false, false),
            (P::CAR_LOAD, true, true),
            (P::LANDING_DOOR_STATUS, true, false),
            (P::FLOOR_TEXT, true, false),
            (P::ENERGY_METER, true, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::FLOOR_NUMBER, true, false),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_elevator_property_metadata_is_exact() {
    let fresh: [fn() -> (Box<dyn BACnetObject>, ObjectType); 3] = [
        || {
            (
                Box::new(ElevatorGroupObject::new(7, "EG-7").unwrap()),
                ObjectType::ELEVATOR_GROUP,
            )
        },
        || {
            (
                Box::new(EscalatorObject::new(7, "ESC-7").unwrap()),
                ObjectType::ESCALATOR,
            )
        },
        || {
            (
                Box::new(LiftObject::new(7, "LIFT-7", 2).unwrap()),
                ObjectType::LIFT,
            )
        },
    ];
    for make in fresh {
        let expected = expected_rows(make().1);
        for configured in [false, true] {
            for out_of_service in [false, true] {
                let (mut object, kind) = make();
                if configured {
                    object
                        .write_property(
                            P::DESCRIPTION,
                            None,
                            PropertyValue::CharacterString("long elevator label".repeat(100)),
                            None,
                        )
                        .unwrap();
                }
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
                let required = object.required_properties();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, kind);
                assert!(!support.createable);
                assert!(support.deleteable);
                let rows: Vec<_> = support
                    .supported_properties
                    .iter()
                    .map(|row| {
                        assert!(row.access.readable);
                        (row.property_id, row.access.optional, row.access.writable)
                    })
                    .collect();
                assert_eq!(
                    rows, expected,
                    "{kind:?}, configured={configured}, OOS={out_of_service}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    required.as_ref()
                );
            }
        }
    }
}
