use super::super::*;
use super::{make_dest_device, make_time};
use std::borrow::Cow;

enum RecipientListBehavior {
    Unavailable,
    Invalid,
}

struct LookupTestNotificationClass {
    oid: ObjectIdentifier,
    behavior: RecipientListBehavior,
}

impl LookupTestNotificationClass {
    fn new(behavior: RecipientListBehavior) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            behavior,
        }
    }
}

impl BACnetObject for LookupTestNotificationClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "lookup-test-notification-class"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(1)),
            p if p == PropertyIdentifier::RECIPIENT_LIST => match self.behavior {
                RecipientListBehavior::Unavailable => Err(Error::Protocol {
                    class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                    code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                }),
                // A malformed complete list with no valid destination prefix.
                RecipientListBehavior::Invalid => Ok(PropertyValue::ApplicationData(vec![0x5E])),
            },
            _ => Err(Error::Protocol {
                class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::RECIPIENT_LIST,
        ])
    }
}

fn lookup(db: &ObjectDatabase) -> RecipientLookupOutcome {
    lookup_notification_recipients(db, 1, EventTransition::ToOffnormal, 0x01, &make_time(12, 0))
}

#[test]
fn lookup_distinguishes_missing_notification_class_and_wrappers_remain_empty() {
    let db = ObjectDatabase::new();

    assert!(matches!(
        lookup(&db),
        RecipientLookupOutcome::NotificationClassMissing
    ));
    assert!(get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    )
    .is_empty());
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        Some(Vec::new())
    );
}

#[test]
fn lookup_distinguishes_unavailable_recipient_list_and_preserves_strict_mapping() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LookupTestNotificationClass::new(
        RecipientListBehavior::Unavailable,
    )))
    .unwrap();

    assert!(matches!(
        lookup(&db),
        RecipientLookupOutcome::RecipientListUnavailable
    ));
    assert!(get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    )
    .is_empty());
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        Some(Vec::new())
    );
}

#[test]
fn lookup_distinguishes_invalid_full_list_without_delivering_a_prefix() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LookupTestNotificationClass::new(
        RecipientListBehavior::Invalid,
    )))
    .unwrap();

    assert!(matches!(
        lookup(&db),
        RecipientLookupOutcome::RecipientListInvalid
    ));
    assert!(get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    )
    .is_empty());
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        None
    );
}

#[test]
fn lookup_distinguishes_zero_configured_destinations_and_wrappers_remain_empty() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(NotificationClass::new(1, "NC-1").unwrap()))
        .unwrap();

    assert!(matches!(
        lookup(&db),
        RecipientLookupOutcome::NoConfiguredDestinations
    ));
    assert!(get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    )
    .is_empty());
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        Some(Vec::new())
    );
}

#[test]
fn lookup_distinguishes_configured_but_ineligible_destinations() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let mut destination = make_dest_device(10);
    destination.valid_days = 0x02;
    nc.add_destination(destination);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(nc)).unwrap();

    assert!(matches!(
        lookup(&db),
        RecipientLookupOutcome::NoMatchingDestinations
    ));
    assert!(get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    )
    .is_empty());
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        Some(Vec::new())
    );
}

#[test]
fn lookup_returns_selected_device_recipient_as_a_match_for_both_wrappers() {
    let destination = make_dest_device(10);
    let expected = (
        destination.recipient.clone(),
        destination.process_identifier,
        destination.issue_confirmed_notifications,
    );
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(destination);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(nc)).unwrap();

    let RecipientLookupOutcome::Matched(recipients) = lookup(&db) else {
        panic!("eligible device recipient must be a lookup success");
    };
    assert_eq!(recipients, vec![expected.clone()]);
    assert_eq!(
        get_notification_recipients(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        vec![expected.clone()]
    );
    assert_eq!(
        get_notification_recipients_strict(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x01,
            &make_time(12, 0),
        ),
        Some(vec![expected])
    );
}
