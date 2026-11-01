use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;

#[test]
fn pics_timer_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in declaration order;
    // PICS sorts by property ID. Table 12-75: Timer_State and Timer_Running
    // are writable because Present_Value is, which also requires the range
    // (#2192).
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, true, false),
        // Writable out of service (footnote 1).
        (P::RELIABILITY, true, true),
        (P::OUT_OF_SERVICE, true, true),
        (P::TIMER_STATE, false, true),
        (P::TIMER_RUNNING, false, true),
        (P::UPDATE_TIME, true, false),
        (P::LAST_STATE_CHANGE, true, false),
        (P::EXPIRATION_TIME, true, false),
        (P::INITIAL_TIMEOUT, true, false),
        (P::DEFAULT_TIMEOUT, true, true),
        (P::MIN_PRES_VALUE, false, false),
        (P::MAX_PRES_VALUE, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = TimerObject::new(7, "TMR-7").unwrap();
            if configured {
                object
                    .write_property(
                        P::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("long timer label".repeat(100)),
                        None,
                    )
                    .unwrap();
                object.set_pres_value_range(1_000, 600_000).unwrap();
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(5000), None)
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
            assert_eq!(support.object_type, ObjectType::TIMER);
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
                "configured={configured}, OOS={out_of_service}"
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
