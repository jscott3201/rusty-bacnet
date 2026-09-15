use super::*;

#[test]
fn pics_loop_program_notification_class_property_metadata_is_exact() {
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_objects::{loop_obj::LoopObject, program::ProgramObject, traits::BACnetObject};
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    let loop_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, false),
        (P::SETPOINT, false, true),
        (P::PROPORTIONAL_CONSTANT, true, true),
        (P::INTEGRAL_CONSTANT, true, true),
        (P::DERIVATIVE_CONSTANT, true, true),
        (P::OUTPUT_UNITS, false, false),
        (P::UPDATE_INTERVAL, true, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::RELIABILITY, true, true),
        (P::OUT_OF_SERVICE, false, true),
        (P::CONTROLLED_VARIABLE_REFERENCE, false, true),
        (P::MANIPULATED_VARIABLE_REFERENCE, false, true),
        (P::SETPOINT_REFERENCE, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    let program_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PROGRAM_STATE, false, true),
        (P::PROGRAM_CHANGE, false, true),
        (P::REASON_FOR_HALT, true, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, true, false),
        (P::PROPERTY_LIST, false, false),
    ];
    let notification_class_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, true, false),
        (P::EVENT_STATE, true, false),
        (P::OUT_OF_SERVICE, true, true),
        (P::RELIABILITY, true, false),
        (P::NOTIFICATION_CLASS, false, true),
        (P::PRIORITY, false, false),
        (P::ACK_REQUIRED, false, false),
        (P::RECIPIENT_LIST, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    for out_of_service in [false, true] {
        let objects: [Box<dyn BACnetObject>; 3] = [
            Box::new(LoopObject::new(1, "LOOP-1", 62).unwrap()),
            Box::new(ProgramObject::new(1, "PRG-1").unwrap()),
            Box::new(NotificationClass::new(1, "NC-1").unwrap()),
        ];
        for mut object in objects {
            let kind = object.object_identifier().object_type();
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
            let expected = if kind == ObjectType::LOOP {
                loop_rows.as_slice()
            } else if kind == ObjectType::PROGRAM {
                program_rows.as_slice()
            } else {
                notification_class_rows.as_slice()
            };
            assert_eq!(rows, expected, "{kind:?}, OOS={out_of_service}");
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}
