use super::*;
use bacnet_objects::{
    lighting::{BinaryLightingOutputObject, LightingOutputObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<(P, bool, bool)> {
    // Independent (identifier, optional, writable) rows in projection order.
    match kind {
        ObjectType::LIGHTING_OUTPUT => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, true),
            (P::TRACKING_VALUE, false, false),
            (P::LIGHTING_COMMAND, false, true),
            (P::LIGHTING_COMMAND_DEFAULT_PRIORITY, false, true),
            (P::IN_PROGRESS, false, false),
            (P::BLINK_WARN_ENABLE, false, true),
            (P::EGRESS_TIME, false, true),
            (P::EGRESS_ACTIVE, false, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::PRIORITY_ARRAY, false, true),
            (P::RELINQUISH_DEFAULT, false, true),
            (P::DEFAULT_FADE_TIME, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, true),
            (P::BLINK_WARN_ENABLE, false, true),
            (P::EGRESS_TIME, false, true),
            (P::EGRESS_ACTIVE, false, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, true, false),
            (P::PRIORITY_ARRAY, false, true),
            (P::RELINQUISH_DEFAULT, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_lighting_property_metadata_is_exact() {
    let fresh: [fn() -> (Box<dyn BACnetObject>, ObjectType); 2] = [
        || {
            (
                Box::new(LightingOutputObject::new(7, "LO-7").unwrap()),
                ObjectType::LIGHTING_OUTPUT,
            )
        },
        || {
            (
                Box::new(BinaryLightingOutputObject::new(7, "BLO-7").unwrap()),
                ObjectType::BINARY_LIGHTING_OUTPUT,
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
                            PropertyValue::CharacterString("long lighting label".repeat(100)),
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
