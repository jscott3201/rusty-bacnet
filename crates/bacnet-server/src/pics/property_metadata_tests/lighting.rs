use super::*;
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<PropertyRow> {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
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
            (P::PRIORITY_ARRAY, false, false),
            (P::RELINQUISH_DEFAULT, false, true),
            (P::DEFAULT_FADE_TIME, false, true),
            (P::DEFAULT_RAMP_RATE, false, true),
            (P::DEFAULT_STEP_INCREMENT, false, true),
            (P::CURRENT_COMMAND_PRIORITY, false, false),
            (P::COV_INCREMENT, true, true),
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
            (P::PRIORITY_ARRAY, false, false),
            (P::RELINQUISH_DEFAULT, false, true),
            (P::CURRENT_COMMAND_PRIORITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_lighting_property_metadata_is_exact() {
    let fresh: [FreshObject; 2] = [
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

/// The PICS rows of a database holding only `object`, and its required
/// properties as the PICS should list them.
fn pics_rows(
    object: Box<dyn bacnet_objects::traits::BACnetObject>,
) -> (Vec<PropertyRow>, Vec<PropertyIdentifier>) {
    let required = sorted_required(object.required_properties().as_ref());
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    let rows = pics.supported_object_types[0]
        .supported_properties
        .iter()
        .map(|row| (row.property_id, row.access.optional, row.access.writable))
        .collect();
    (rows, required)
}

/// The required rows among `rows`, as the PICS orders them.
fn required_of(rows: &[PropertyRow]) -> Vec<PropertyIdentifier> {
    rows.iter()
        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
        .collect()
}

#[test]
fn pics_lighting_output_lists_its_trims_once_set() {
    // A high trim alone (#1528): High_End_Trim is optional, and the
    // Trim_Fade_Time it brings is required by the table's footnote.
    let mut object = LightingOutputObject::new(7, "LO-7").unwrap();
    object.set_high_end_trim(Some(90.0)).unwrap();
    let mut expected = expected_rows(ObjectType::LIGHTING_OUTPUT);
    expected.extend([
        (P::HIGH_END_TRIM, true, true),
        (P::TRIM_FADE_TIME, false, true),
    ]);
    let (rows, required) = pics_rows(Box::new(object));
    assert_eq!(rows, sorted_rows(&expected));
    assert_eq!(required_of(&rows), required);
    // Both trims.
    let mut object = LightingOutputObject::new(7, "LO-7").unwrap();
    object.set_low_end_trim(Some(10.0)).unwrap();
    object.set_high_end_trim(Some(90.0)).unwrap();
    expected.push((P::LOW_END_TRIM, true, true));
    let (rows, required) = pics_rows(Box::new(object));
    assert_eq!(rows, sorted_rows(&expected));
    assert_eq!(required_of(&rows), required);
}

#[test]
fn pics_lighting_outputs_list_their_colour_links_as_required_and_writable() {
    use bacnet_objects::lighting::{ColorLink, ColorOverride};
    // Each row is required while present (the tables' footnotes, #1527).
    let colour = ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap();
    let rows_for = |overridable: bool| {
        let mut rows = vec![(P::COLOR_REFERENCE, false, true)];
        if overridable {
            rows.extend([
                (P::COLOR_OVERRIDE, false, true),
                (P::OVERRIDE_COLOR_REFERENCE, false, true),
            ]);
        }
        rows
    };
    for overridable in [false, true] {
        let link = ColorLink {
            color_override: overridable.then_some(ColorOverride {
                active: false,
                reference: colour,
            }),
            ..ColorLink::new(colour)
        };
        let mut lo = LightingOutputObject::new(7, "LO-7").unwrap();
        lo.set_color_link(Some(link)).unwrap();
        let mut blo = BinaryLightingOutputObject::new(7, "BLO-7").unwrap();
        blo.set_color_link(Some(link)).unwrap();
        let objects: [(Box<dyn bacnet_objects::traits::BACnetObject>, ObjectType); 2] = [
            (Box::new(lo), ObjectType::LIGHTING_OUTPUT),
            (Box::new(blo), ObjectType::BINARY_LIGHTING_OUTPUT),
        ];
        for (object, kind) in objects {
            let mut expected = expected_rows(kind);
            expected.extend(rows_for(overridable));
            let (rows, required) = pics_rows(object);
            assert_eq!(rows, sorted_rows(&expected), "{kind:?}");
            assert_eq!(required_of(&rows), required);
        }
    }
}
