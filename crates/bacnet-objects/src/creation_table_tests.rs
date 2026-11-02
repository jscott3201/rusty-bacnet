//! The type-level lifecycle table (#2179): creation-only properties, their
//! initializers and deleteability, over the whole catalogue. Commit
//! 1e3fd4924 checked the same table against every catalogue object's own
//! answers before the BACnetObject hooks went; this pins those answers.

use super::*;
use crate::catalogue::representatives;
use PropertyIdentifier as P;

/// Each built-in type with creation-only properties, and the properties.
const EXPECTED: &[(ObjectType, &[PropertyIdentifier])] = &[
    (ObjectType::ANALOG_INPUT, &[P::UNITS]),
    (ObjectType::ANALOG_OUTPUT, &[P::UNITS]),
    (ObjectType::ANALOG_VALUE, &[P::UNITS]),
    (ObjectType::MULTI_STATE_INPUT, &[P::NUMBER_OF_STATES]),
    (ObjectType::MULTI_STATE_OUTPUT, &[P::NUMBER_OF_STATES]),
    (ObjectType::MULTI_STATE_VALUE, &[P::NUMBER_OF_STATES]),
    (ObjectType::INTEGER_VALUE, &[P::UNITS]),
    (ObjectType::POSITIVE_INTEGER_VALUE, &[P::UNITS]),
    (ObjectType::LARGE_ANALOG_VALUE, &[P::UNITS]),
    (ObjectType::CALENDAR, &[P::OBJECT_NAME]),
    (ObjectType::SCHEDULE, &[P::OBJECT_NAME]),
    (
        ObjectType::NOTIFICATION_CLASS,
        &[P::OBJECT_NAME, P::PRIORITY, P::ACK_REQUIRED],
    ),
    (ObjectType::TREND_LOG, &[P::OBJECT_NAME, P::BUFFER_SIZE]),
    (
        ObjectType::EVENT_ENROLLMENT,
        &[P::OBJECT_PROPERTY_REFERENCE],
    ),
    (
        ObjectType::STRUCTURED_VIEW,
        &[
            P::OBJECT_NAME,
            P::NODE_TYPE,
            P::NODE_SUBTYPE,
            P::SUBORDINATE_LIST,
            P::SUBORDINATE_ANNOTATIONS,
        ],
    ),
];

#[test]
fn each_catalogue_type_has_its_creation_only_properties_and_deleteability() {
    let catalogue = representatives();
    for object in &catalogue {
        let kind = object.object_identifier().object_type();
        let expected = EXPECTED
            .iter()
            .find(|(listed, _)| *listed == kind)
            .map_or(&[][..], |(_, properties)| *properties);
        assert_eq!(
            builtin_creation_only(kind).properties(),
            expected,
            "{kind:?}"
        );
        assert_eq!(
            builtin_deleteable(kind),
            !matches!(kind, ObjectType::DEVICE | ObjectType::NETWORK_PORT),
            "{kind:?}"
        );
    }
    for (kind, _) in EXPECTED {
        assert!(builtin_constructor(*kind).is_some(), "{kind:?}");
    }
    // A type with no entry: no creation-only property, and deleteable.
    let vendor = ObjectType::from_raw(600);
    assert!(builtin_creation_only(vendor).properties().is_empty());
    assert!(builtin_deleteable(vendor));
}

#[test]
fn an_initializer_sets_its_own_types_objects_and_refuses_any_other() {
    let mut db = ObjectDatabase::new();
    for object in representatives() {
        db.add(object).unwrap();
    }
    let schedule = ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap();
    let calendar = ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap();
    let name = PropertyValue::CharacterString("renamed".into());
    let initializer = builtin_creation_only(ObjectType::SCHEDULE);
    initializer
        .initialize(&mut db, &schedule, P::OBJECT_NAME, name.clone())
        .unwrap();
    assert_eq!(db.get(&schedule).unwrap().object_name(), "renamed");
    // The Calendar takes a name the same way, but not through the
    // Schedule's initializer.
    let denied = format!("{:?}", crate::common::write_access_denied_error());
    for refused in [
        initializer.initialize(&mut db, &calendar, P::OBJECT_NAME, name.clone()),
        CreationOnly::NONE.initialize(&mut db, &schedule, P::OBJECT_NAME, name.clone()),
        initializer.initialize(
            &mut db,
            &ObjectIdentifier::new(ObjectType::SCHEDULE, 9).unwrap(),
            P::OBJECT_NAME,
            name,
        ),
    ] {
        assert_eq!(format!("{:?}", refused.unwrap_err()), denied);
    }
    assert_ne!(db.get(&calendar).unwrap().object_name(), "renamed");
}

#[test]
fn each_entrys_creation_only_answers_come_from_the_type_its_constructor_builds() {
    // An entry's `O` and its constructor are written apart; a mismatch would
    // route every creation-only initial value to a downcast that fails.
    for kind in builtin_createable_types() {
        let entry = entry(kind).unwrap();
        let object = (entry.constructor.unwrap())(1, "made".into(), &[]).unwrap();
        let object: &(dyn BACnetObject + 'static) = object.as_ref();
        let built = object
            .as_stored_any(crate::traits::ObjectStorageAccess(()))
            .type_id();
        assert_eq!(built, (entry.creation_only.type_id)(), "{kind:?}");
    }
}

/// `BUILTIN` is a `const`, filled in at build time, so a coverage run sees
/// `creatable` and `fixed` as never called. Calling them here at run time
/// checks they build what the table holds (#2183).
#[test]
fn the_table_entries_are_what_their_builders_make_at_run_time() {
    use core::hint::black_box;
    let held = entry(ObjectType::DEVICE).unwrap();
    let device = fixed(black_box(ObjectType::DEVICE));
    assert_eq!(device.object_type, held.object_type);
    assert!(device.constructor.is_none() && held.constructor.is_none());
    assert_eq!(device.deleteable, held.deleteable);
    assert_eq!(
        device.creation_only.properties(),
        held.creation_only.properties()
    );

    let held = entry(ObjectType::ANALOG_VALUE).unwrap();
    let analog = creatable::<AnalogValueObject>(
        black_box(ObjectType::ANALOG_VALUE),
        held.constructor.unwrap(),
    );
    assert_eq!(analog.object_type, held.object_type);
    assert!(analog.constructor.is_some());
    assert_eq!(analog.deleteable, held.deleteable);
    assert_eq!(analog.creation_only.properties(), [P::UNITS]);
    assert_eq!(
        (analog.creation_only.type_id)(),
        (held.creation_only.type_id)()
    );
}
