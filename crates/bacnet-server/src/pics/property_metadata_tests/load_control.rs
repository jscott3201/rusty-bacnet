use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{load_control::LoadControlObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;

#[test]
fn pics_load_control_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
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
        // Table 12-32 has no Out_Of_Service (#1064).
        (P::STATUS_FLAGS, true, false),
        (P::RELIABILITY, true, false),
        (P::EVENT_STATE, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
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
                    PropertyValue::ApplicationData(vec![0x09, 50]),
                    None,
                )
                .unwrap();
            object
                .write_property(P::SHED_DURATION, None, PropertyValue::Unsigned(3600), None)
                .unwrap();
        }
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
        assert_eq!(rows, sorted_rows(&expected), "configured={configured}");
        assert_eq!(
            rows.iter()
                .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                .collect::<Vec<_>>(),
            sorted_required(required.as_ref())
        );
    }
}
