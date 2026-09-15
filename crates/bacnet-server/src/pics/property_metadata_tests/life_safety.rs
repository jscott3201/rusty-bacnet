use super::*;
use bacnet_objects::{
    life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject},
    traits::BACnetObject,
};
use PropertyIdentifier as P;

#[test]
fn pics_life_safety_point_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::OBJECT_TYPE, false, false),
        (P::DESCRIPTION, true, true),
        (P::PRESENT_VALUE, false, false),
        (P::MODE, false, true),
        (P::SILENCED, false, false),
        (P::OPERATION_EXPECTED, false, false),
        (P::TRACKING_VALUE, false, false),
        (P::MEMBER_OF, true, false),
        (P::DIRECT_READING, true, true),
        (P::MAINTENANCE_REQUIRED, true, true),
        (P::EVENT_STATE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = LifeSafetyPointObject::new(7, "LSP-7").unwrap();
            if configured {
                object.set_description("long life safety label".repeat(100));
                object.set_direct_reading(42.5);
                object.add_member(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 9).unwrap());
            }
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    bacnet_types::primitives::PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::LIFE_SAFETY_POINT);
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

#[test]
fn pics_life_safety_zone_property_metadata_is_exact() {
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::OBJECT_TYPE, false, false),
        (P::DESCRIPTION, true, true),
        (P::PRESENT_VALUE, false, false),
        (P::MODE, false, true),
        (P::SILENCED, false, false),
        (P::OPERATION_EXPECTED, false, false),
        (P::ZONE_MEMBERS, false, false),
        (P::EVENT_STATE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = LifeSafetyZoneObject::new(7, "LSZ-7").unwrap();
            if configured {
                object.set_description("long life safety label".repeat(100));
                object.add_zone_member(
                    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap(),
                );
            }
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    bacnet_types::primitives::PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::LIFE_SAFETY_ZONE);
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
