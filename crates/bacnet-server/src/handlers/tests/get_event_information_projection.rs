use std::borrow::Cow;

use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_services::alarm_event::{GetEventInformationAck, GetEventInformationRequest};
use bacnet_types::primitives::{Date, Time};

use super::*;

struct ProjectionFixture {
    oid: ObjectIdentifier,
    name: String,
    advertised: Vec<PropertyIdentifier>,
    values: Vec<(PropertyIdentifier, PropertyValue)>,
}

impl ProjectionFixture {
    fn summary(instance: u32) -> Self {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
        Self {
            oid,
            name: format!("SUMMARY-{instance}"),
            advertised: vec![
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::ACKED_TRANSITIONS,
                PropertyIdentifier::EVENT_TIME_STAMPS,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyIdentifier::EVENT_ENABLE,
                PropertyIdentifier::NOTIFICATION_CLASS,
            ],
            values: vec![
                (
                    PropertyIdentifier::EVENT_STATE,
                    PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
                ),
                (
                    PropertyIdentifier::ACKED_TRANSITIONS,
                    transition_bits(0b111),
                ),
                (
                    PropertyIdentifier::EVENT_TIME_STAMPS,
                    PropertyValue::List(vec![
                        PropertyValue::Unsigned(1),
                        PropertyValue::Unsigned(2),
                        PropertyValue::Unsigned(3),
                    ]),
                ),
                (
                    PropertyIdentifier::NOTIFY_TYPE,
                    PropertyValue::Enumerated(0),
                ),
                (PropertyIdentifier::EVENT_ENABLE, transition_bits(0b111)),
                (
                    PropertyIdentifier::NOTIFICATION_CLASS,
                    PropertyValue::Unsigned(42),
                ),
            ],
        }
    }

    fn notification_class(
        instance: u32,
        class_number: Option<PropertyValue>,
        priority: Option<PropertyValue>,
    ) -> Self {
        let mut values = Vec::new();
        if let Some(class_number) = class_number {
            values.push((PropertyIdentifier::NOTIFICATION_CLASS, class_number));
        }
        if let Some(priority) = priority {
            values.push((PropertyIdentifier::PRIORITY, priority));
        }
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, instance).unwrap(),
            name: format!("CLASS-{instance}"),
            advertised: vec![
                PropertyIdentifier::NOTIFICATION_CLASS,
                PropertyIdentifier::PRIORITY,
            ],
            values,
        }
    }

    fn set(&mut self, property: PropertyIdentifier, value: PropertyValue) {
        self.values.retain(|(candidate, _)| *candidate != property);
        self.values.push((property, value));
    }

    fn remove(&mut self, property: PropertyIdentifier) {
        self.values.retain(|(candidate, _)| *candidate != property);
    }

    fn advertise(&mut self, property: PropertyIdentifier) {
        if !self.advertised.contains(&property) {
            self.advertised.push(property);
        }
    }
}

impl BACnetObject for ProjectionFixture {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.values
            .iter()
            .find(|(candidate, _)| *candidate == property)
            .map(|(_, value)| value.clone())
            .ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(self.advertised.clone())
    }
}

fn transition_bits(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bacnet_types::bitstring::pack_octet(bits)],
    }
}

fn timestamp_value(timestamp: &BACnetTimeStamp) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_timestamp_choice(&mut encoded, timestamp).unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

fn add_class(db: &mut ObjectDatabase, instance: u32, priorities: [u8; 3]) {
    let mut class = NotificationClass::new(instance, format!("NC-{instance}")).unwrap();
    class.notification_class = 42;
    class.priority = priorities;
    db.add(Box::new(class)).unwrap();
}

fn request(cursor: Option<ObjectIdentifier>) -> BytesMut {
    let mut encoded = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: cursor,
    }
    .encode(&mut encoded);
    encoded
}

fn response(
    db: &ObjectDatabase,
    cursor: Option<ObjectIdentifier>,
    budget: Option<usize>,
) -> Result<(GetEventInformationAck, usize), Error> {
    let mut encoded = BytesMut::new();
    handle_get_event_information_with_budget(db, &request(cursor), &mut encoded, budget)?;
    Ok((GetEventInformationAck::decode(&encoded)?, encoded.len()))
}

fn encoded_ack_len(summaries: &[bacnet_services::alarm_event::EventSummary], more: bool) -> usize {
    let mut encoded = BytesMut::new();
    GetEventInformationAck {
        list_of_event_summaries: summaries.to_vec(),
        more_events: more,
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.len()
}

fn assert_operational_problem(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::DEVICE.to_raw() as u32
                && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32
    ));
}

#[test]
fn selection_uses_state_acknowledgments_and_detection_not_event_enable() {
    let mut db = ObjectDatabase::new();
    let mut normal_unacked = ProjectionFixture::summary(1);
    normal_unacked.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
    );
    normal_unacked.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        transition_bits(0b110),
    );
    normal_unacked.set(PropertyIdentifier::EVENT_ENABLE, transition_bits(0));
    db.add(Box::new(normal_unacked)).unwrap();

    let mut normal_acked = ProjectionFixture::summary(2);
    normal_acked.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
    );
    db.add(Box::new(normal_acked)).unwrap();
    db.add(Box::new(ProjectionFixture::summary(3))).unwrap();
    add_class(&mut db, 99, [4, 80, 255]);

    let (ack, _) = response(&db, None, None).unwrap();
    let instances: Vec<_> = ack
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert_eq!(instances, vec![1, 3]);
    assert_eq!(ack.list_of_event_summaries[0].event_enable, 0);
    assert_eq!(
        ack.list_of_event_summaries[0].event_priorities,
        [4, 80, 255]
    );
}

#[test]
fn detection_false_excludes_but_absent_is_eligible_and_malformed_errors() {
    let mut db = ObjectDatabase::new();
    let mut disabled = ProjectionFixture::summary(1);
    disabled.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    disabled.set(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    disabled.set(PropertyIdentifier::EVENT_STATE, PropertyValue::Unsigned(1));
    disabled.remove(PropertyIdentifier::EVENT_TIME_STAMPS);
    disabled.set(
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(999),
    );
    db.add(Box::new(disabled)).unwrap();
    db.add(Box::new(ProjectionFixture::summary(2))).unwrap();
    add_class(&mut db, 99, [1, 2, 3]);
    let (ack, _) = response(&db, None, None).unwrap();
    assert_eq!(ack.list_of_event_summaries.len(), 1);
    assert_eq!(
        ack.list_of_event_summaries[0]
            .object_identifier
            .instance_number(),
        2
    );

    let mut malformed_db = ObjectDatabase::new();
    let mut malformed = ProjectionFixture::summary(3);
    malformed.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    malformed.set(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Unsigned(1),
    );
    malformed_db.add(Box::new(malformed)).unwrap();
    add_class(&mut malformed_db, 99, [1, 2, 3]);
    assert_operational_problem(response(&malformed_db, None, None).unwrap_err());

    let mut unreadable_db = ObjectDatabase::new();
    let mut unreadable = ProjectionFixture::summary(4);
    unreadable.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    unreadable.remove(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    unreadable_db.add(Box::new(unreadable)).unwrap();
    add_class(&mut unreadable_db, 99, [1, 2, 3]);
    assert_operational_problem(response(&unreadable_db, None, None).unwrap_err());
}

#[test]
fn malformed_required_projection_fields_error_and_non_event_objects_are_skipped() {
    let malformed = [
        (PropertyIdentifier::EVENT_STATE, PropertyValue::Unsigned(1)),
        (
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0xe0],
            },
        ),
        (PropertyIdentifier::NOTIFY_TYPE, PropertyValue::Unsigned(0)),
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe1],
            },
        ),
        (
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyValue::Enumerated(42),
        ),
        (
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyValue::List(vec![PropertyValue::Unsigned(1); 2]),
        ),
    ];

    for (property, value) in malformed {
        let mut db = ObjectDatabase::new();
        let mut object = ProjectionFixture::summary(1);
        object.set(property, value);
        db.add(Box::new(object)).unwrap();
        add_class(&mut db, 99, [1, 2, 3]);
        assert_operational_problem(response(&db, None, None).unwrap_err());
    }

    let mut unreadable_db = ObjectDatabase::new();
    let mut unreadable = ProjectionFixture::summary(1);
    unreadable.remove(PropertyIdentifier::NOTIFY_TYPE);
    unreadable_db.add(Box::new(unreadable)).unwrap();
    add_class(&mut unreadable_db, 99, [1, 2, 3]);
    assert_operational_problem(response(&unreadable_db, None, None).unwrap_err());

    let mut skipped_db = ObjectDatabase::new();
    let skipped = ProjectionFixture {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        name: "NON-EVENT".into(),
        advertised: vec![PropertyIdentifier::EVENT_STATE],
        values: Vec::new(),
    };
    skipped_db.add(Box::new(skipped)).unwrap();
    let (ack, _) = response(&skipped_db, None, None).unwrap();
    assert!(ack.list_of_event_summaries.is_empty());
}

#[test]
fn notification_class_lookup_requires_one_exact_well_formed_match() {
    let mut valid_with_unrelated_malformed = ObjectDatabase::new();
    valid_with_unrelated_malformed
        .add(Box::new(ProjectionFixture::summary(1)))
        .unwrap();
    add_class(&mut valid_with_unrelated_malformed, 99, [1, 2, 3]);
    for malformed in [
        ProjectionFixture::notification_class(1, Some(PropertyValue::Enumerated(42)), None),
        ProjectionFixture::notification_class(2, None, None),
        ProjectionFixture::notification_class(
            3,
            Some(PropertyValue::Unsigned(u64::from(u32::MAX) + 1)),
            None,
        ),
    ] {
        valid_with_unrelated_malformed
            .add(Box::new(malformed))
            .unwrap();
    }
    let (ack, _) = response(&valid_with_unrelated_malformed, None, None).unwrap();
    assert_eq!(ack.list_of_event_summaries.len(), 1);
    assert_eq!(ack.list_of_event_summaries[0].event_priorities, [1, 2, 3]);

    let scenarios = [
        "missing",
        "duplicate",
        "malformed-number",
        "missing-number",
        "out-of-u32-number",
        "missing-priority",
        "malformed-priority",
        "wrong-length",
        "over-255",
    ];
    for scenario in scenarios {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ProjectionFixture::summary(1))).unwrap();
        match scenario {
            "missing" => {}
            "duplicate" => {
                add_class(&mut db, 1, [1, 2, 3]);
                add_class(&mut db, 2, [4, 5, 6]);
            }
            "malformed-number" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Enumerated(42)),
                    Some(PropertyValue::List(vec![PropertyValue::Unsigned(1); 3])),
                )))
                .unwrap(),
            "missing-number" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    None,
                    Some(PropertyValue::List(vec![PropertyValue::Unsigned(1); 3])),
                )))
                .unwrap(),
            "out-of-u32-number" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Unsigned(u64::from(u32::MAX) + 1)),
                    Some(PropertyValue::List(vec![PropertyValue::Unsigned(1); 3])),
                )))
                .unwrap(),
            "missing-priority" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Unsigned(42)),
                    None,
                )))
                .unwrap(),
            "malformed-priority" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Unsigned(42)),
                    Some(PropertyValue::Enumerated(1)),
                )))
                .unwrap(),
            "wrong-length" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Unsigned(42)),
                    Some(PropertyValue::List(vec![PropertyValue::Unsigned(1); 2])),
                )))
                .unwrap(),
            "over-255" => db
                .add(Box::new(ProjectionFixture::notification_class(
                    1,
                    Some(PropertyValue::Unsigned(42)),
                    Some(PropertyValue::List(vec![
                        PropertyValue::Unsigned(1),
                        PropertyValue::Unsigned(256),
                        PropertyValue::Unsigned(3),
                    ])),
                )))
                .unwrap(),
            _ => unreachable!(),
        }
        assert_operational_problem(response(&db, None, None).unwrap_err());
    }
}

#[test]
fn timestamp_choices_keep_coordinate_order_and_malformed_values_error() {
    let expected = [
        BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        }),
        BACnetTimeStamp::SequenceNumber(65535),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 8,
                day: 31,
                day_of_week: 1,
            },
            time: Time {
                hour: 5,
                minute: 6,
                second: 7,
                hundredths: 8,
            },
        },
    ];
    let mut db = ObjectDatabase::new();
    let mut object = ProjectionFixture::summary(1);
    object.set(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyValue::List(
            expected
                .clone()
                .map(|timestamp| timestamp_value(&timestamp))
                .to_vec(),
        ),
    );
    db.add(Box::new(object)).unwrap();
    add_class(&mut db, 99, [1, 2, 3]);
    let (ack, _) = response(&db, None, None).unwrap();
    assert_eq!(ack.list_of_event_summaries[0].event_timestamps, expected);

    let mut malformed_db = ObjectDatabase::new();
    let mut malformed = ProjectionFixture::summary(1);
    malformed.set(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyValue::List(vec![PropertyValue::CharacterString("bad".into()); 3]),
    );
    malformed_db.add(Box::new(malformed)).unwrap();
    add_class(&mut malformed_db, 99, [1, 2, 3]);
    assert_operational_problem(response(&malformed_db, None, None).unwrap_err());
}

#[test]
fn byte_budget_selects_maximal_prefix_on_both_sides_of_twenty_five() {
    let mut db = ObjectDatabase::new();
    for instance in (1..=30).rev() {
        let mut object = ProjectionFixture::summary(instance);
        if instance == 4 {
            object.set(
                PropertyIdentifier::EVENT_TIME_STAMPS,
                PropertyValue::List(vec![
                    timestamp_value(&BACnetTimeStamp::DateTime {
                        date: Date {
                            year: 126,
                            month: 9,
                            day: 1,
                            day_of_week: 2,
                        },
                        time: Time {
                            hour: 1,
                            minute: 2,
                            second: 3,
                            hundredths: 4,
                        },
                    }),
                    PropertyValue::Unsigned(2),
                    PropertyValue::Unsigned(3),
                ]),
            );
        }
        db.add(Box::new(object)).unwrap();
    }
    add_class(&mut db, 99, [1, 2, 3]);

    let (full, _) = response(&db, None, None).unwrap();
    assert_eq!(full.list_of_event_summaries.len(), 30);
    let three_budget = encoded_ack_len(&full.list_of_event_summaries[..3], true);
    assert!(encoded_ack_len(&full.list_of_event_summaries[..4], true) > three_budget);
    let (first, first_len) = response(&db, None, Some(three_budget)).unwrap();
    assert_eq!(first.list_of_event_summaries.len(), 3);
    assert_eq!(first_len, three_budget);
    assert!(first.more_events);

    let cursor = first
        .list_of_event_summaries
        .last()
        .unwrap()
        .object_identifier;
    let (second, _) = response(&db, Some(cursor), Some(three_budget)).unwrap();
    let resumed: Vec<_> = second
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert!(!resumed.is_empty());
    assert_eq!(resumed[0], 4);
    let first_instances: Vec<_> = first
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert_eq!(first_instances, vec![1, 2, 3]);
    assert_eq!(
        first_instances
            .into_iter()
            .chain(resumed.iter().copied())
            .collect::<Vec<_>>(),
        (1..=3 + resumed.len() as u32).collect::<Vec<_>>()
    );

    let twenty_seven_budget = encoded_ack_len(&full.list_of_event_summaries[..27], true);
    let (large, large_len) = response(&db, None, Some(twenty_seven_budget)).unwrap();
    assert_eq!(large.list_of_event_summaries.len(), 27);
    assert_eq!(large_len, twenty_seven_budget);
    assert!(large.more_events);

    let (oversized_first, oversized_len) = response(&db, None, Some(1)).unwrap();
    assert_eq!(oversized_first.list_of_event_summaries.len(), 1);
    assert!(oversized_first.more_events);
    assert!(oversized_len > 1);
}

#[test]
fn ordering_cursor_successor_and_empty_result_are_deterministic() {
    let empty = ObjectDatabase::new();
    let (ack, _) = response(&empty, None, None).unwrap();
    assert!(ack.list_of_event_summaries.is_empty());
    assert!(!ack.more_events);

    let mut db = ObjectDatabase::new();
    for instance in [9, 1, 5] {
        db.add(Box::new(ProjectionFixture::summary(instance)))
            .unwrap();
    }
    add_class(&mut db, 99, [1, 2, 3]);
    let deleted_cursor = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 4).unwrap();
    let (ack, _) = response(&db, Some(deleted_cursor), None).unwrap();
    let instances: Vec<_> = ack
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert_eq!(instances, vec![5, 9]);

    let present_cursor = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    let (ack, _) = response(&db, Some(present_cursor), None).unwrap();
    assert_eq!(ack.list_of_event_summaries.len(), 1);
    assert_eq!(
        ack.list_of_event_summaries[0]
            .object_identifier
            .instance_number(),
        9
    );
}
