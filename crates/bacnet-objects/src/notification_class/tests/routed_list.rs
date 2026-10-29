//! What routing takes from a Notification Class it did not build: no list
//! past [`MAX_RECIPIENT_LIST_DESTINATIONS`] (#1124) and no flat form (#1125).
//! Either way no destination is selected, so a transition never reaches part
//! of a list.

use super::super::*;
use super::{make_dest_device, make_time};
use std::borrow::Cow;

const CAP: usize = MAX_RECIPIENT_LIST_DESTINATIONS;

/// A custom Notification Class 1 serving `recipient_list` as it is.
struct CustomClass {
    oid: ObjectIdentifier,
    recipient_list: PropertyValue,
}

impl CustomClass {
    fn serving(recipient_list: PropertyValue) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            recipient_list,
        }
    }

    /// Serving `count` framed device destinations, process identifiers 1 to
    /// `count`.
    fn framed(count: usize) -> Self {
        let destinations: Vec<_> = (1..=count as u32)
            .map(|process_identifier| BACnetDestination {
                process_identifier,
                ..make_dest_device(10)
            })
            .collect();
        let mut bytes = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_destination_list(&mut bytes, &destinations).unwrap();
        Self::serving(PropertyValue::ApplicationData(bytes.to_vec()))
    }
}

impl BACnetObject for CustomClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "custom-notification-class"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(1)),
            p if p == PropertyIdentifier::RECIPIENT_LIST => Ok(self.recipient_list.clone()),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(common::protocol_error(
            bacnet_types::enums::ErrorClass::PROPERTY,
            bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::RECIPIENT_LIST,
        ])
    }
}

/// Selected recipients: the recipient, process identifier and confirmed flag.
type Selected = Vec<(BACnetRecipient, u32, bool)>;

/// The lookup outcome and both wrappers' results for class 1.
fn route(class: CustomClass) -> (RecipientLookupOutcome, Selected, Option<Selected>) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    let noon = make_time(12, 0);
    let args = (1, EventTransition::ToOffnormal, DaysOfWeek::MONDAY);
    (
        lookup_notification_recipients(&db, args.0, args.1, args.2, &noon),
        get_notification_recipients(&db, args.0, args.1, args.2, &noon),
        get_notification_recipients_strict(&db, args.0, args.1, args.2, &noon),
    )
}

#[test]
fn custom_class_at_the_cap_routes_every_destination() {
    let (outcome, plain, strict) = route(CustomClass::framed(CAP));
    let RecipientLookupOutcome::Matched(recipients) = outcome else {
        panic!("a list at the cap routes");
    };
    assert_eq!(recipients.len(), CAP);
    assert_eq!(plain, recipients);
    assert_eq!(strict, Some(recipients));
}

#[test]
fn custom_class_past_the_cap_routes_none_of_its_list() {
    for count in [CAP + 1, 10 * CAP] {
        let class = CustomClass::framed(count);
        let served = class.recipient_list.clone();
        let (outcome, plain, strict) = route(class);
        assert_eq!(
            outcome,
            RecipientLookupOutcome::RecipientListTooLong,
            "{count}"
        );
        assert!(plain.is_empty(), "{count}");
        assert_eq!(strict, None, "{count}");
        assert!(
            filter_recipient_list(
                &served,
                EventTransition::ToOffnormal,
                DaysOfWeek::MONDAY,
                &make_time(12, 0),
            )
            .is_empty()
        );
    }
}

#[test]
fn custom_class_serving_the_flat_form_routes_nothing() {
    let flat_entry = PropertyValue::List(vec![
        PropertyValue::BitString {
            unused_bits: 1,
            data: vec![0b1111_1110],
        },
        PropertyValue::Time(make_time(0, 0)),
        PropertyValue::Time(make_time(23, 59)),
        PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap()),
        PropertyValue::Unsigned(1),
        PropertyValue::Boolean(true),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        },
    ]);
    let (outcome, plain, strict) =
        route(CustomClass::serving(PropertyValue::List(vec![flat_entry])));
    assert_eq!(outcome, RecipientLookupOutcome::RecipientListInvalid);
    assert!(plain.is_empty());
    assert_eq!(strict, None);
}
