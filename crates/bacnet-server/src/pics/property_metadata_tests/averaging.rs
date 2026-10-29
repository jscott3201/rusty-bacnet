use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{averaging::AveragingObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;

#[test]
fn pics_averaging_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        // Table 12-5 has no Present_Value or status rows (#1064).
        (P::MINIMUM_VALUE, false, false),
        (P::MAXIMUM_VALUE, false, false),
        (P::AVERAGE_VALUE, false, false),
        // Attempted_Samples takes a write of zero, and the window rows take
        // writes (#1092); each one resets the window.
        (P::ATTEMPTED_SAMPLES, false, true),
        (P::VALID_SAMPLES, false, false),
        (P::OBJECT_PROPERTY_REFERENCE, false, true),
        (P::WINDOW_INTERVAL, false, true),
        (P::WINDOW_SAMPLES, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        let mut object = AveragingObject::new(7, "AVG-7").unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    PropertyValue::CharacterString("long averaging label".repeat(100)),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::OBJECT_PROPERTY_REFERENCE,
                    None,
                    // [0] analog-input 1, [1] present-value (#1182).
                    PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]),
                    None,
                )
                .unwrap();
            object
                .write_property(P::WINDOW_SAMPLES, None, PropertyValue::Unsigned(60), None)
                .unwrap();
            object.add_sample(10.0).unwrap();
            object.add_sample(20.0).unwrap();
            object.add_sample(30.0).unwrap();
        }
        let required = object.required_properties();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        assert_eq!(pics.supported_object_types.len(), 1);
        let support = &pics.supported_object_types[0];
        assert_eq!(support.object_type, ObjectType::AVERAGING);
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
