//! Before/after table test for #2177: the element codec the list services
//! chose by (object type, property) before the metadata rows carried value
//! types, frozen, against the codec the write policy and the standard value
//! types give (every built-in row resolves to the standard type). Equal over every pair, so a request to an
//! object that exists or one that doesn't decodes its elements as before.

use super::*;
use bacnet_objects::property_metadata::standard_value_type;

#[test]
fn standard_value_types_pick_the_old_codec_on_every_pair() {
    let mut differences = Vec::new();
    for object_type in (0..=1023).map(ObjectType::from_raw) {
        for property in (0..=1023).map(PropertyIdentifier::from_raw).chain(
            PropertyIdentifier::ALL_NAMED
                .iter()
                .map(|&(_, property)| property)
                .filter(|property| property.to_raw() > 1023),
        ) {
            let new = ListCodec::for_target(
                write_policy(object_type, property).typed_list_elements,
                standard_value_type(object_type, property),
            );
            let old = old_codec(object_type, property);
            if new != old {
                differences.push(format!("{object_type:?} {property:?}: {new:?} was {old:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// Every property the write policy reads as typed elements is a list whose
/// datatype has a codec here, so none of them degrades to application
/// values.
#[test]
fn every_typed_list_policy_entry_has_a_codec() {
    let mut typed = 0;
    for object_type in (0..=1023).map(ObjectType::from_raw) {
        for property in (0..=1023).map(PropertyIdentifier::from_raw).chain(
            PropertyIdentifier::ALL_NAMED
                .iter()
                .map(|&(_, property)| property)
                .filter(|property| property.to_raw() > 1023),
        ) {
            if !write_policy(object_type, property).typed_list_elements {
                continue;
            }
            typed += 1;
            let value_type = standard_value_type(object_type, property);
            assert!(
                value_type.is_some_and(|v| v.is_list()),
                "{object_type:?} {property:?} is not a list"
            );
            assert_ne!(
                ListCodec::for_target(true, value_type),
                ListCodec::Values,
                "{object_type:?} {property:?} has no typed codec"
            );
        }
    }
    assert_eq!(typed, 5);
}

/// The codec choice before #2177, frozen.
fn old_codec(object_type: ObjectType, property: PropertyIdentifier) -> ListCodec {
    match property {
        PropertyIdentifier::RECIPIENT_LIST
            if matches!(
                object_type,
                ObjectType::NOTIFICATION_CLASS | ObjectType::NOTIFICATION_FORWARDER
            ) =>
        {
            ListCodec::Destinations
        }
        PropertyIdentifier::DATE_LIST if object_type == ObjectType::CALENDAR => {
            ListCodec::CalendarEntries
        }
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
            if object_type == ObjectType::SCHEDULE =>
        {
            ListCodec::References
        }
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS
            if object_type == ObjectType::NOTIFICATION_FORWARDER =>
        {
            ListCodec::Subscriptions
        }
        _ => ListCodec::Values,
    }
}
