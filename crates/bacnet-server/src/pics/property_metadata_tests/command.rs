use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{command::CommandObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;

#[test]
fn pics_command_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::IN_PROCESS, false, false),
        (P::ALL_WRITES_SUCCESSFUL, false, false),
        (P::ACTION, false, false),
        // Table 12-12 has no Out_Of_Service (#1064).
        (P::STATUS_FLAGS, true, false),
        (P::RELIABILITY, true, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        let mut object = CommandObject::new(7, "CMD-7").unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    PropertyValue::CharacterString("long command label".repeat(100)),
                    None,
                )
                .unwrap();
            // Present_Value 3 selects the last of three empty lists.
            object
                .set_action(vec![
                    bacnet_types::constructed::BACnetActionList::default();
                    3
                ])
                .unwrap();
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(3), None)
                .unwrap();
        }
        let required = object.required_properties();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        assert_eq!(pics.supported_object_types.len(), 1);
        let support = &pics.supported_object_types[0];
        assert_eq!(support.object_type, ObjectType::COMMAND);
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
