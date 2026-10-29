use super::*;
use PropertyIdentifier as P;
use bacnet_objects::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use bacnet_types::primitives::PropertyValue;

fn expected_rows(kind: ObjectType) -> Vec<PropertyRow> {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    match kind {
        ObjectType::ELEVATOR_GROUP => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::MACHINE_ROOM_ID, false, false),
            (P::GROUP_ID, false, true),
            (P::GROUP_MEMBERS, false, false),
            (P::GROUP_MODE, true, true),
            (P::LANDING_CALLS, true, false),
            (P::LANDING_CALL_CONTROL, true, true),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ESCALATOR => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::OBJECT_TYPE, false, false),
            (P::DESCRIPTION, true, true),
            (P::STATUS_FLAGS, false, false),
            (P::ELEVATOR_GROUP, false, false),
            (P::GROUP_ID, false, false),
            (P::INSTALLATION_ID, false, false),
            (P::POWER_MODE, true, true),
            (P::OPERATION_DIRECTION, false, true),
            (P::ESCALATOR_MODE, true, true),
            (P::ENERGY_METER, true, true),
            (P::ENERGY_METER_REF, true, false),
            (P::RELIABILITY, true, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::FAULT_SIGNALS, true, true),
            (P::PASSENGER_ALARM, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::OBJECT_TYPE, false, false),
            (P::DESCRIPTION, true, true),
            (P::STATUS_FLAGS, false, false),
            (P::ELEVATOR_GROUP, false, false),
            (P::GROUP_ID, false, false),
            (P::INSTALLATION_ID, false, false),
            (P::FLOOR_TEXT, true, false),
            // The per-door arrays and car-state rows are writable only while
            // Out_Of_Service is TRUE (#1035, #1052); PICS lists them writable.
            (P::ASSIGNED_LANDING_CALLS, true, true),
            (P::MAKING_CAR_CALL, true, true),
            (P::REGISTERED_CAR_CALL, true, true),
            (P::CAR_POSITION, false, true),
            (P::CAR_MOVING_DIRECTION, false, true),
            (P::CAR_ASSIGNED_DIRECTION, true, true),
            (P::CAR_DOOR_STATUS, false, true),
            (P::CAR_DOOR_COMMAND, true, true),
            (P::CAR_DOOR_ZONE, true, true),
            (P::CAR_MODE, true, true),
            (P::CAR_LOAD, true, true),
            (P::CAR_LOAD_UNITS, true, false),
            (P::NEXT_STOPPING_FLOOR, true, true),
            (P::PASSENGER_ALARM, false, true),
            (P::ENERGY_METER, true, true),
            (P::ENERGY_METER_REF, true, false),
            (P::RELIABILITY, true, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::CAR_DRIVE_STATUS, true, true),
            (P::FAULT_SIGNALS, false, true),
            (P::LANDING_DOOR_STATUS, true, true),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_elevator_property_metadata_is_exact() {
    let fresh: [FreshObject; 3] = [
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
                // Elevator Group has no Out_Of_Service (Table 12-76).
                if kind != ObjectType::ELEVATOR_GROUP {
                    object
                        .write_property(
                            P::OUT_OF_SERVICE,
                            None,
                            PropertyValue::Boolean(out_of_service),
                            None,
                        )
                        .unwrap();
                }
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
                    rows,
                    sorted_rows(&expected),
                    "{kind:?}, configured={configured}, OOS={out_of_service}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    sorted_required(required.as_ref())
                );
            }
        }
    }
}
