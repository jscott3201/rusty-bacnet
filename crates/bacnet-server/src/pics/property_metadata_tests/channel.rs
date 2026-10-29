use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{channel::ChannelObject, traits::BACnetObject};

#[test]
fn pics_channel_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in Table 12-62 order;
    // PICS sorts by property ID.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, true),
        (P::OBJECT_TYPE, false, false),
        (P::DESCRIPTION, true, true),
        (P::PRESENT_VALUE, false, true),
        (P::LAST_PRIORITY, false, false),
        (P::WRITE_STATUS, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::RELIABILITY, true, true),
        (P::OUT_OF_SERVICE, false, true),
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, false, true),
        (P::EXECUTION_DELAY, true, true),
        (P::ALLOW_GROUP_DELAY_INHIBIT, true, true),
        (P::CHANNEL_NUMBER, false, true),
        (P::CONTROL_GROUPS, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    let object = ChannelObject::new(7, "CH-7", 11).unwrap();
    let required = object.required_properties();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.supported_object_types.len(), 1);
    let support = &pics.supported_object_types[0];
    assert_eq!(support.object_type, ObjectType::CHANNEL);
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
    assert_eq!(rows, sorted_rows(&expected));
    assert_eq!(
        rows.iter()
            .filter_map(|&(p, optional, _)| (!optional).then_some(p))
            .collect::<Vec<_>>(),
        sorted_required(required.as_ref())
    );
}
