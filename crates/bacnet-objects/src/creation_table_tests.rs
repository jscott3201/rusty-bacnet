//! The type-level lifecycle table against the objects' own answers (#2179):
//! creation-only properties, their initializers and deleteability, over the
//! whole catalogue.

use super::*;
use crate::catalogue::representatives;

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    for object in representatives() {
        db.add(object).unwrap();
    }
    db
}

/// The initial values each initializer is tried with: a name, counts, a
/// unit inside and outside BACnetEngineeringUnits, NULL, a wrong datatype,
/// a list and a three-bit transition set.
fn trial_values() -> Vec<PropertyValue> {
    vec![
        PropertyValue::CharacterString("renamed".into()),
        PropertyValue::CharacterString(String::new()),
        PropertyValue::Unsigned(3),
        PropertyValue::Unsigned(0),
        PropertyValue::Enumerated(62),
        PropertyValue::Enumerated(70_000),
        PropertyValue::Null,
        PropertyValue::Real(1.0),
        PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ]),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        },
    ]
}

/// The properties tried on every type: each creation-only property of a
/// built-in type, and some that none takes only at creation.
const TRIAL_PROPERTIES: &[PropertyIdentifier] = &[
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::UNITS,
    PropertyIdentifier::NUMBER_OF_STATES,
    PropertyIdentifier::PRIORITY,
    PropertyIdentifier::ACK_REQUIRED,
    PropertyIdentifier::BUFFER_SIZE,
    PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
    PropertyIdentifier::NODE_TYPE,
    PropertyIdentifier::NODE_SUBTYPE,
    PropertyIdentifier::SUBORDINATE_LIST,
    PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::DESCRIPTION,
];

#[test]
fn the_table_answers_as_each_catalogue_object_did() {
    let db = database();
    let mut seen = 0;
    for (oid, object) in db.iter_objects() {
        let kind = oid.object_type();
        assert_eq!(
            builtin_creation_only(kind).properties(),
            object.creation_only_properties(),
            "{kind:?}"
        );
        assert_eq!(builtin_deleteable(kind), object.is_deleteable(), "{kind:?}");
        for property in object.creation_only_properties() {
            assert!(TRIAL_PROPERTIES.contains(property), "{kind:?} {property:?}");
        }
        seen += 1;
    }
    assert_eq!(seen, representatives().len());
}

#[test]
fn each_initializer_answers_and_changes_as_the_object_did() {
    let mut applied = 0;
    for property in TRIAL_PROPERTIES.iter().copied() {
        for value in trial_values() {
            let mut old = database();
            let mut new = database();
            for oid in old.list_objects() {
                let kind = oid.object_type();
                let before = old.get_mut(&oid).unwrap();
                let was = before.initialize_property(property, value.clone());
                let is =
                    builtin_creation_only(kind).initialize(&mut new, &oid, property, value.clone());
                applied += usize::from(is.is_ok());
                assert_eq!(
                    format!("{is:?}"),
                    format!("{was:?}"),
                    "{kind:?} {property:?} {value:?}"
                );
                assert_eq!(
                    format!("{:?}", new.get(&oid).unwrap().read_property(property, None)),
                    format!("{:?}", old.get(&oid).unwrap().read_property(property, None)),
                    "{kind:?} {property:?} {value:?}"
                );
            }
        }
    }
    // The trials reach the initializers' success paths too: names, units,
    // counts, priorities and transition sets.
    assert!(applied >= 20, "{applied}");
}
