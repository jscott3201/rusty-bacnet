use super::*;
use bacnet_objects::{load_control::LoadControlObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

#[test]
fn pics_load_control_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, false),
        (P::REQUESTED_SHED_LEVEL, false, true),
        (P::EXPECTED_SHED_LEVEL, false, false),
        (P::ACTUAL_SHED_LEVEL, false, false),
        (P::SHED_DURATION, false, true),
        (P::START_TIME, false, false),
        (P::STATUS_FLAGS, true, false),
        (P::OUT_OF_SERVICE, true, true),
        (P::RELIABILITY, true, false),
        (P::EVENT_STATE, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = LoadControlObject::new(7, "LC-7").unwrap();
            if configured {
                object
                    .write_property(
                        P::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("long load control label".repeat(100)),
                        None,
                    )
                    .unwrap();
                object
                    .write_property(
                        P::REQUESTED_SHED_LEVEL,
                        None,
                        PropertyValue::List(vec![PropertyValue::Unsigned(50)]),
                        None,
                    )
                    .unwrap();
                object
                    .write_property(P::SHED_DURATION, None, PropertyValue::Unsigned(3600), None)
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
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::LOAD_CONTROL);
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
                "configured={configured}, OOS={out_of_service}"
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
