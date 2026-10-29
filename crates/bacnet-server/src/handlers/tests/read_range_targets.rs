//! ReadRange serves only a BACnetLIST (#1025; Clause 15.8). The decision
//! follows the property's datatype, as for the list services (#999), not the
//! shape of the value read: a whole array and a BACnetDateTime both read as
//! lists, and a list held framed reads as one block of bytes, which ReadRange
//! splits into its elements.

use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::elevator::ElevatorGroupObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::value_types::DateTimeValueObject;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;

const POSITION: Option<RangeSpec> = Some(RangeSpec::ByPosition {
    reference_index: 1,
    count: 2,
});
const SEQUENCE: Option<RangeSpec> = Some(RangeSpec::BySequenceNumber {
    reference_seq: 1,
    count: 2,
});

fn by_time() -> Option<RangeSpec> {
    Some(RangeSpec::ByTime {
        reference_time: (DATE, time(0)),
        count: 2,
    })
}

fn add(db: &mut ObjectDatabase, object: impl BACnetObject + 'static) -> ObjectIdentifier {
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    oid
}

fn assert_refused(
    result: Result<ReadRangeAck, Error>,
    class: ErrorClass,
    code: ErrorCode,
    context: &str,
) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: got_class,
            code: got_code,
        })) => assert_eq!(
            (got_class, got_code),
            (class, code),
            "{context}: expected {class:?}/{code:?}"
        ),
        other => panic!("{context}: expected {class:?}/{code:?}, got {other:?}"),
    }
}

/// Every case must refuse with SERVICES / PROPERTY_IS_NOT_A_LIST, whatever
/// the range form.
fn assert_not_a_list(
    db: &ObjectDatabase,
    cases: &[(ObjectIdentifier, PropertyIdentifier, Option<u32>)],
) {
    for &(oid, property, index) in cases {
        for range in [None, POSITION, SEQUENCE, by_time()] {
            let result = call_with_index(db, oid, property, index, range.clone());
            if !matches!(&result, Err(Error::Remote(RemoteError::Error { class, code }))
                if *class == ErrorClass::SERVICES
                    && *code == ErrorCode::PROPERTY_IS_NOT_A_LIST)
            {
                panic!("{oid:?} {property:?} index {index:?} {range:?}: {result:?}");
            }
        }
    }
}

fn msv_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    msv.set_alarm_values(vec![1, 2, 3]);
    let oid = add(&mut db, msv);
    (db, oid)
}

fn destination(process_identifier: u32) -> BACnetDestination {
    BACnetDestination {
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
            second: 0,
            hundredths: 0,
        },
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, process_identifier).unwrap(),
        ),
        process_identifier,
        issue_confirmed_notifications: process_identifier.is_multiple_of(2),
        transitions: EventTransitionBits::all(),
    }
}

fn encoded_destination(process_identifier: u32) -> PropertyValue {
    let mut encoded = BytesMut::new();
    bacnet_encoding::constructed::encode_destination(
        &mut encoded,
        &destination(process_identifier),
    )
    .unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// An object that reads one property as a fixed value and calls it a list.
struct FixedList {
    oid: ObjectIdentifier,
    name: &'static str,
    property: PropertyIdentifier,
    value: PropertyValue,
}

impl BACnetObject for FixedList {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == self.property {
            Ok(self.value.clone())
        } else {
            Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            ))
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![self.property])
    }

    fn is_list_property(&self, property: PropertyIdentifier) -> bool {
        property == self.property
    }
}

#[test]
fn read_range_refuses_whole_arrays_and_array_elements_as_not_a_list() {
    let (mut db, msv) = msv_db();
    let device = add(
        &mut db,
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("DEV-1"),
            ..Default::default()
        })
        .unwrap(),
    );
    let schedule = add(
        &mut db,
        ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap(),
    );
    assert_not_a_list(
        &db,
        &[
            // Whole arrays read as lists but are not BACnetLISTs.
            (msv, PropertyIdentifier::STATE_TEXT, None),
            (msv, PropertyIdentifier::PROPERTY_LIST, None),
            (msv, PropertyIdentifier::PRIORITY_ARRAY, None),
            (device, PropertyIdentifier::OBJECT_LIST, None),
            (schedule, PropertyIdentifier::WEEKLY_SCHEDULE, None),
            // Neither is an element, even one that reads as several values.
            (msv, PropertyIdentifier::STATE_TEXT, Some(1)),
            (msv, PropertyIdentifier::PRIORITY_ARRAY, Some(16)),
            (schedule, PropertyIdentifier::WEEKLY_SCHEDULE, Some(1)),
        ],
    );
}

#[test]
fn read_range_refuses_scalars_and_constructed_values_as_not_a_list() {
    let (mut db, msv) = msv_db();
    let ai = add(
        &mut db,
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    );
    let dtv = add(&mut db, DateTimeValueObject::new(1, "DTV-1").unwrap());
    let group = add(&mut db, ElevatorGroupObject::new(1, "EG-1").unwrap());
    let schedule = add(
        &mut db,
        ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap(),
    );
    assert_not_a_list(
        &db,
        &[
            (ai, PropertyIdentifier::PRESENT_VALUE, None),
            (msv, PropertyIdentifier::PRESENT_VALUE, None),
            (msv, PropertyIdentifier::OUT_OF_SERVICE, None),
            (msv, PropertyIdentifier::OBJECT_NAME, None),
            // A BACnetDateTime reads as its two members.
            (dtv, PropertyIdentifier::PRESENT_VALUE, None),
            // Single constructed values held framed.
            (group, PropertyIdentifier::LANDING_CALL_CONTROL, None),
            (schedule, PropertyIdentifier::EFFECTIVE_PERIOD, None),
        ],
    );
}

#[test]
fn read_range_target_errors_follow_clause_order() {
    let (db, msv) = msv_db();
    let missing = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 99).unwrap();
    let vendor = PropertyIdentifier::from_raw(9999);
    // Object, then property, then the array index, then the list kind, and
    // only then whether the items support the range form.
    let ladder = [
        (
            missing,
            PropertyIdentifier::ALARM_VALUES,
            None,
            None,
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            msv,
            vendor,
            None,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            msv,
            vendor,
            Some(1),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            msv,
            PropertyIdentifier::PRESENT_VALUE,
            Some(1),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            msv,
            PropertyIdentifier::ALARM_VALUES,
            Some(1),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            msv,
            PropertyIdentifier::STATE_TEXT,
            Some(99),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (
            msv,
            PropertyIdentifier::STATE_TEXT,
            None,
            SEQUENCE,
            ErrorClass::SERVICES,
            ErrorCode::PROPERTY_IS_NOT_A_LIST,
        ),
        (
            msv,
            PropertyIdentifier::STATE_TEXT,
            None,
            by_time(),
            ErrorClass::SERVICES,
            ErrorCode::PROPERTY_IS_NOT_A_LIST,
        ),
        (
            msv,
            PropertyIdentifier::ALARM_VALUES,
            None,
            SEQUENCE,
            ErrorClass::PROPERTY,
            ErrorCode::LIST_ITEM_NOT_NUMBERED,
        ),
        (
            msv,
            PropertyIdentifier::ALARM_VALUES,
            None,
            by_time(),
            ErrorClass::PROPERTY,
            ErrorCode::LIST_ITEM_NOT_TIMESTAMPED,
        ),
    ];
    for (oid, property, index, range, class, code) in ladder {
        let context = format!("{oid:?} {property:?} index {index:?} {range:?}");
        assert_refused(
            call_with_index(&db, oid, property, index, range),
            class,
            code,
            &context,
        );
    }
}

#[test]
fn read_range_pages_plain_lists_by_position() {
    let (db, msv) = msv_db();
    let all = call(&db, msv, PropertyIdentifier::ALARM_VALUES, None).unwrap();
    assert_ack(&all, &unsigned_items(&[1, 2, 3]), (true, true, false), None);
    let back = call(
        &db,
        msv,
        PropertyIdentifier::ALARM_VALUES,
        Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: -2,
        }),
    )
    .unwrap();
    assert_ack(&back, &unsigned_items(&[1, 2]), (true, false, false), None);
}

#[test]
fn read_range_splits_framed_recipient_list_into_destinations() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    for process_identifier in 1..=3 {
        nc.add_destination(destination(process_identifier)).unwrap();
    }
    let nc = add(&mut db, nc);
    let property = PropertyIdentifier::RECIPIENT_LIST;
    let items: Vec<_> = (1..=3).map(encoded_destination).collect();

    // The items are the frame ReadProperty returns, cut at each destination.
    let all = call(&db, nc, property, None).unwrap();
    assert_ack(&all, &items, (true, true, false), None);
    let mut framed = BytesMut::new();
    let destinations: Vec<_> = (1..=3).map(destination).collect();
    bacnet_encoding::constructed::encode_destination_list(&mut framed, &destinations).unwrap();
    assert_eq!(all.item_data, framed.to_vec());

    let second = call(
        &db,
        nc,
        property,
        Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: 1,
        }),
    )
    .unwrap();
    assert_ack(&second, &items[1..2], (false, false, false), None);
    let last_two = call(
        &db,
        nc,
        property,
        Some(RangeSpec::ByPosition {
            reference_index: 3,
            count: -2,
        }),
    )
    .unwrap();
    assert_ack(&last_two, &items[1..], (false, true, false), None);
    assert_refused(
        call(&db, nc, property, SEQUENCE),
        ErrorClass::PROPERTY,
        ErrorCode::LIST_ITEM_NOT_NUMBERED,
        "Recipient_List by sequence",
    );
}

#[test]
fn read_range_splits_framed_schedule_references() {
    let mut db = ObjectDatabase::new();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap();
    let av = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    // The first carries the optional array index, so each element's length
    // differs and only the reference codec finds the boundary.
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new_indexed(
            av,
            PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
            8,
        ))
        .unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            av,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    let schedule = add(&mut db, schedule);
    let ack = call(
        &db,
        schedule,
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: 1,
        }),
    )
    .unwrap();
    let mut second = BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(
        &mut second,
        &BACnetObjectPropertyReference::new(av, PropertyIdentifier::PRESENT_VALUE.to_raw()),
    );
    assert_ack(
        &ack,
        &[PropertyValue::ApplicationData(second.to_vec())],
        (false, true, false),
        None,
    );
}

#[test]
fn read_range_splits_subscribed_recipients_into_subscriptions() {
    use bacnet_encoding::constructed::{
        encode_event_notification_subscription, encode_event_notification_subscription_list,
    };
    use bacnet_types::constructed::BACnetEventNotificationSubscription;

    // A device and an address recipient encode to different lengths, so only
    // the subscription codec finds each boundary.
    let subscriptions = [
        BACnetEventNotificationSubscription {
            recipient: BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
            ),
            process_identifier: 1,
            issue_confirmed_notifications: true,
            time_remaining: 30,
        },
        BACnetEventNotificationSubscription {
            recipient: BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
                network_number: 4,
                mac_address: bacnet_types::MacAddr::from_slice(&[10, 0, 0, 2, 0xBA, 0xC0]),
            }),
            process_identifier: 2,
            issue_confirmed_notifications: false,
            time_remaining: 1440,
        },
    ];
    let mut framed = BytesMut::new();
    encode_event_notification_subscription_list(&mut framed, &subscriptions).unwrap();
    let mut forwarder = crate::server::test_forwarder::TestForwarder::new(1);
    forwarder
        .subscribed_recipients
        .write(PropertyValue::ApplicationData(framed.to_vec()))
        .unwrap();
    let mut db = ObjectDatabase::new();
    let forwarder = add(&mut db, forwarder);
    let items: Vec<_> = subscriptions
        .iter()
        .map(|subscription| {
            let mut encoded = BytesMut::new();
            encode_event_notification_subscription(&mut encoded, subscription).unwrap();
            PropertyValue::ApplicationData(encoded.to_vec())
        })
        .collect();
    let property = PropertyIdentifier::SUBSCRIBED_RECIPIENTS;

    let all = call(&db, forwarder, property, None).unwrap();
    assert_ack(&all, &items, (true, true, false), None);
    assert_eq!(all.item_data, framed.to_vec());
    let second = call(
        &db,
        forwarder,
        property,
        Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: 1,
        }),
    )
    .unwrap();
    assert_ack(&second, &items[1..], (false, true, false), None);
}

#[test]
fn read_range_standalone_device_cov_lists_page_as_read_property_reads_them() {
    // Without a running server the Device holds no COV subscriptions, and
    // standalone ReadProperty returns both lists empty. ReadRange splits the
    // same empty frame with the element walkers, so it pages no items rather
    // than refusing; the live lists are paged in the server's wire tests.
    let mut db = ObjectDatabase::new();
    let device = add(
        &mut db,
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("DEV-1"),
            ..Default::default()
        })
        .unwrap(),
    );
    for property in [
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
    ] {
        assert_eq!(
            db.get(&device)
                .unwrap()
                .read_property(property, None)
                .unwrap(),
            PropertyValue::ApplicationData(Vec::new())
        );
        for range in [None, POSITION] {
            let ack = call(&db, device, property, range.clone()).unwrap();
            assert_ack(&ack, &[], (false, false, false), None);
        }
        assert_refused(
            call(&db, device, property, SEQUENCE),
            ErrorClass::PROPERTY,
            ErrorCode::LIST_ITEM_NOT_NUMBERED,
            &format!("{property:?} by sequence"),
        );
    }
}

#[test]
fn read_range_refuses_lists_it_cannot_split() {
    let mut db = ObjectDatabase::new();
    let mut truncated = encoded_destination(1);
    if let PropertyValue::ApplicationData(bytes) = &mut truncated {
        bytes.pop();
    }
    let cases = [
        // A vendor list held framed, and a framed Recipient_List whose stored
        // frame does not decode.
        (
            add(
                &mut db,
                FixedList {
                    oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
                    name: "vendor-framed",
                    property: PropertyIdentifier::from_raw(600),
                    value: encoded_destination(1),
                },
            ),
            PropertyIdentifier::from_raw(600),
        ),
        (
            add(
                &mut db,
                FixedList {
                    oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
                    name: "truncated-recipients",
                    property: PropertyIdentifier::RECIPIENT_LIST,
                    value: truncated,
                },
            ),
            PropertyIdentifier::RECIPIENT_LIST,
        ),
        // A list property whose value has another shape.
        (
            add(
                &mut db,
                FixedList {
                    oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap(),
                    name: "vendor-scalar",
                    property: PropertyIdentifier::from_raw(601),
                    value: PropertyValue::Unsigned(1),
                },
            ),
            PropertyIdentifier::from_raw(601),
        ),
    ];
    for (oid, property) in cases {
        for range in [None, POSITION] {
            assert_refused(
                call(&db, oid, property, range.clone()),
                ErrorClass::SERVICES,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                &format!("{oid:?} {property:?} {range:?}"),
            );
        }
    }
}
