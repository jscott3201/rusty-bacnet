use std::borrow::Cow;

use super::super::*;

#[test]
fn projection_resolves_direct_instance_selects_coordinate_and_matches_membership() {
    let recipient = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap());
    let mut destination = super::make_dest_device(9);
    destination.valid_days = 0;
    destination.from_time = super::make_time(23, 0);
    destination.to_time = super::make_time(1, 0);
    destination.transitions = 0;
    destination.issue_confirmed_notifications = false;
    destination.process_identifier = 77;

    let mut class = NotificationClass::new(7, "NC-7").unwrap();
    class.notification_class = 40;
    class.priority = [11, 22, 33];
    class.recipient_list = vec![destination];
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    let mut unrelated = NotificationClass::new(40, "NC-40").unwrap();
    unrelated.notification_class = 7;
    unrelated.priority = [1, 2, 3];
    db.add(Box::new(unrelated)).unwrap();

    for (transition, priority) in [
        (EventTransition::ToOffnormal, 11),
        (EventTransition::ToFault, 22),
        (EventTransition::ToNormal, 33),
    ] {
        let projection =
            resolve_enrollment_summary_class_internal(&db, 7, transition, Some((&recipient, 77)))
                .unwrap();
        assert_eq!(projection.priority, priority);
        assert!(projection.enrollment_member);
    }
    assert!(
        !resolve_enrollment_summary_class_internal(
            &db,
            7,
            EventTransition::ToOffnormal,
            Some((&recipient, 78)),
        )
        .unwrap()
        .enrollment_member
    );
}

#[test]
fn projection_reports_missing_direct_instance_even_when_another_class_uses_number() {
    let mut db = ObjectDatabase::new();
    let mut unrelated = NotificationClass::new(40, "NC-40").unwrap();
    unrelated.notification_class = 7;
    db.add(Box::new(unrelated)).unwrap();
    assert_eq!(
        resolve_enrollment_summary_class_internal(&db, 7, EventTransition::ToNormal, None),
        Err(EnrollmentSummaryClassProjectionError::NotificationClassMissing)
    );
}

struct ProjectionFixtureClass {
    oid: ObjectIdentifier,
    notification_class: Option<PropertyValue>,
    recipient_list: Option<PropertyValue>,
}

impl BACnetObject for ProjectionFixtureClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "malformed-recipient-class"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::NOTIFICATION_CLASS => self
                .notification_class
                .clone()
                .ok_or_else(crate::common::unknown_property_error),
            PropertyIdentifier::PRIORITY => Ok(PropertyValue::List(vec![
                PropertyValue::Unsigned(11),
                PropertyValue::Unsigned(22),
                PropertyValue::Unsigned(33),
            ])),
            PropertyIdentifier::RECIPIENT_LIST => self
                .recipient_list
                .clone()
                .ok_or_else(crate::common::unknown_property_error),
            _ => Err(crate::common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(crate::common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

#[test]
fn direct_objects_missing_or_malformed_own_class_property_still_resolve() {
    for notification_class in [None, Some(PropertyValue::Boolean(true))] {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ProjectionFixtureClass {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 7).unwrap(),
            notification_class,
            recipient_list: None,
        }))
        .unwrap();

        assert_eq!(
            resolve_enrollment_summary_class_internal(&db, 7, EventTransition::ToNormal, None)
                .unwrap()
                .priority,
            33
        );
    }
}

#[test]
fn malformed_recipient_list_is_lazy_until_membership_is_requested() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ProjectionFixtureClass {
        oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 7).unwrap(),
        notification_class: None,
        recipient_list: Some(PropertyValue::ApplicationData(vec![0x5e])),
    }))
    .unwrap();

    assert_eq!(
        resolve_enrollment_summary_class_internal(&db, 7, EventTransition::ToNormal, None)
            .unwrap()
            .priority,
        33
    );
    let recipient = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap());
    assert_eq!(
        resolve_enrollment_summary_class_internal(
            &db,
            7,
            EventTransition::ToNormal,
            Some((&recipient, 77)),
        ),
        Err(EnrollmentSummaryClassProjectionError::RecipientListMalformed)
    );
}
