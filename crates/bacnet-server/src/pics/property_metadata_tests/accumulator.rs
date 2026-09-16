use super::*;
use bacnet_objects::{
    accumulator::{AccumulatorObject, PulseConverterObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<(P, bool, bool)> {
    // Independent (identifier, optional, writable) rows in projection order.
    match kind {
        ObjectType::ACCUMULATOR => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, false),
            (P::MAX_PRES_VALUE, false, true),
            (P::SCALE, false, false),
            (P::PRESCALE, true, false),
            (P::PULSE_RATE, true, true),
            (P::UNITS, false, false),
            (P::LIMIT_MONITORING_INTERVAL, true, true),
            (P::STATUS_FLAGS, false, false),
            (P::EVENT_STATE, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::VALUE_BEFORE_CHANGE, true, false),
            (P::VALUE_SET, true, false),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, true),
            (P::UNITS, false, false),
            (P::SCALE_FACTOR, false, true),
            (P::ADJUST_VALUE, false, true),
            (P::COV_INCREMENT, true, true),
            (P::INPUT_REFERENCE, true, true),
            (P::STATUS_FLAGS, false, false),
            (P::EVENT_STATE, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_accumulator_property_metadata_is_exact() {
    let fresh: [fn() -> (Box<dyn BACnetObject>, ObjectType); 2] = [
        || {
            (
                Box::new(AccumulatorObject::new(7, "ACC-7", 95).unwrap()),
                ObjectType::ACCUMULATOR,
            )
        },
        || {
            (
                Box::new(PulseConverterObject::new(7, "PC-7", 62).unwrap()),
                ObjectType::PULSE_CONVERTER,
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
                            PropertyValue::CharacterString("long accumulator label".repeat(100)),
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
