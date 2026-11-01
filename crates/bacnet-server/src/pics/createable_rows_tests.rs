//! Createability in the PICS: a type-level answer from the constructor
//! table (#2172), with a row for each createable type even before its first
//! instance (#2173).

use super::tests::make_pics_config;
use super::*;

#[test]
fn empty_database_lists_each_createable_type_as_a_created_object_describes_it() {
    let db = ObjectDatabase::new();
    let server_config = ServerConfig::default();
    let pics_config = PicsConfig::default();
    let pics = generate_pics(&db, &server_config, &pics_config);
    assert!(!pics.supported_services.is_empty());

    // Createability is the type's, so every type the built-in table builds
    // has a row before its first instance, and nothing else does.
    let types: Vec<ObjectType> = pics
        .supported_object_types
        .iter()
        .map(|support| support.object_type)
        .collect();
    let mut expected: Vec<ObjectType> =
        bacnet_objects::creation::builtin_createable_types().collect();
    expected.sort_by_key(|kind| kind.to_raw());
    assert_eq!(types, expected);

    // Each row is the one a database holding a freshly created object of
    // the type produces, and building it added nothing to the database.
    for support in &pics.supported_object_types {
        let kind = support.object_type;
        assert!(support.createable, "{kind:?}");
        assert!(!support.supported_properties.is_empty(), "{kind:?}");
        let constructor = bacnet_objects::creation::builtin_constructor(kind).unwrap();
        let mut created = ObjectDatabase::new();
        created
            .add(constructor(9, "made".into(), &[]).unwrap())
            .unwrap();
        let with_instance = generate_pics(&created, &server_config, &pics_config);
        let same = with_instance
            .supported_object_types
            .iter()
            .find(|row| row.object_type == kind)
            .unwrap();
        assert_eq!(format!("{support:?}"), format!("{same:?}"), "{kind:?}");
    }
    assert_eq!(db.len(), 0);
}

#[test]
fn pics_createability_of_every_catalogue_type_follows_the_constructor_table() {
    // One table drives both CreateObject and the PICS (#2172), replacing the
    // old pin of Analog Value as not createable.
    use bacnet_objects::creation::builtin_constructor;
    let mut db = ObjectDatabase::new();
    for object in bacnet_objects::catalogue::representatives() {
        db.add(object).unwrap();
    }
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());
    assert_eq!(
        pics.supported_object_types.len(),
        bacnet_objects::device::SUPPORTED_OBJECT_TYPES.len()
    );
    for entry in &pics.supported_object_types {
        assert_eq!(
            entry.createable,
            builtin_constructor(entry.object_type).is_some(),
            "{:?}",
            entry.object_type
        );
    }
    let createable = |kind| {
        pics.supported_object_types
            .iter()
            .find(|entry| entry.object_type == kind)
            .unwrap()
            .createable
    };
    assert!(createable(ObjectType::ANALOG_VALUE));
    assert!(createable(ObjectType::DATETIMEPATTERN_VALUE));
    assert!(!createable(ObjectType::CALENDAR));
    assert!(!createable(ObjectType::DEVICE));
    assert_eq!(
        pics.supported_object_types
            .iter()
            .filter(|entry| entry.createable)
            .count(),
        21
    );
}
