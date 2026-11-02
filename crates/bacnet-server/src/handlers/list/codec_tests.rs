//! Before/after table test for #2177: the element codec the list services
//! chose by (object type, property) before the metadata rows carried value
//! types, frozen, against the codec the standard value types give, which
//! every built-in row resolves to. Equal over every pair, so a request to an
//! object that exists or one that doesn't decodes its elements as before.

use super::*;
use bacnet_objects::property_metadata::standard_value_type;

#[test]
fn standard_value_types_pick_the_old_codec_on_every_pair() {
    let mut differences = Vec::new();
    for object_type in (0..=1023).map(ObjectType::from_raw) {
        for property in (0..=1023).map(PropertyIdentifier::from_raw) {
            let new = ListCodec::for_value_type(standard_value_type(object_type, property));
            let old = old_codec(object_type, property);
            if new != old {
                differences.push(format!("{object_type:?} {property:?}: {new:?} was {old:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
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
