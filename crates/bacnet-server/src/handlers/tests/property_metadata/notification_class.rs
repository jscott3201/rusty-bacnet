use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{notification_class::NotificationClass, traits::BACnetObject};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::{PropertyValue, Time};

#[test]
fn rpm_notification_class_metadata_selectors_preserve_bytes_and_budgets() {
    // Independent legacy-order fixtures, not projections of the metadata under test.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::NOTIFICATION_CLASS,
        P::PRIORITY,
        P::ACK_REQUIRED,
        P::RECIPIENT_LIST,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::NOTIFICATION_CLASS,
        P::PRIORITY,
        P::ACK_REQUIRED,
        P::RECIPIENT_LIST,
    ];
    // Table 12-24 has no Out_Of_Service (#1064).
    let optional = [
        P::DESCRIPTION,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
    ];
    for configured in [false, true] {
        let mut object = NotificationClass::new(7, "NC-7").unwrap();
        if configured {
            object.set_description("long class label".repeat(100));
            object.priority = [12, 34, 56];
            object.ack_required =
                EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL;
            object
                .write_property(
                    P::NOTIFICATION_CLASS,
                    None,
                    PropertyValue::Unsigned(99),
                    None,
                )
                .unwrap();
            object
                .add_destination(BACnetDestination {
                    valid_days: DaysOfWeek::all(),
                    from_time: Time {
                        hour: 0,
                        minute: 0,
                        second: 0,
                        hundredths: 0,
                    },
                    to_time: Time {
                        hour: 23,
                        minute: 59,
                        second: 59,
                        hundredths: 99,
                    },
                    recipient: BACnetRecipient::Device(
                        ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
                    ),
                    process_identifier: 123,
                    issue_confirmed_notifications: true,
                    transitions: EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
                })
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        for (selector, expected) in [
            (P::ALL, all.as_slice()),
            (P::REQUIRED, required.as_slice()),
            (P::OPTIONAL, optional.as_slice()),
            (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
        ] {
            assert_rpm_selector_bytes(&db, oid, selector, expected);
        }
    }
}
