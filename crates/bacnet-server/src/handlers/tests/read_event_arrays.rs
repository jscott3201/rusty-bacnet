use super::*;
use bacnet_encoding::primitives::{decode_timestamp_choice, encode_timestamp_choice};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::primitives::{Date, Time};

const EVENT_ARRAY_PROPERTIES: &[PropertyIdentifier] = &[
    PropertyIdentifier::EVENT_TIME_STAMPS,
    PropertyIdentifier::EVENT_MESSAGE_TEXTS,
];

struct EventArrayFixture {
    oid: ObjectIdentifier,
    timestamps: [PropertyValue; 3],
    messages: [PropertyValue; 3],
}

impl bacnet_objects::traits::BACnetObject for EventArrayFixture {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "EventArrayFixture"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => {
                read_fixed_array(&self.timestamps, array_index)
            }
            p if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS => {
                read_fixed_array(&self.messages, array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
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
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(EVENT_ARRAY_PROPERTIES)
    }
}

fn read_fixed_array(
    values: &[PropertyValue; 3],
    array_index: Option<u32>,
) -> Result<PropertyValue, Error> {
    match array_index {
        None => Ok(PropertyValue::List(values.to_vec())),
        Some(0) => Ok(PropertyValue::Unsigned(3)),
        Some(index @ 1..=3) => Ok(values[index as usize - 1].clone()),
        Some(_) => Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
        }),
    }
}

fn fixture_timestamps() -> [BACnetTimeStamp; 3] {
    [
        BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        }),
        BACnetTimeStamp::SequenceNumber(22),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 7,
                day: 30,
                day_of_week: 4,
            },
            time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
        },
    ]
}

fn event_array_db() -> (ObjectDatabase, ObjectIdentifier, [BACnetTimeStamp; 3]) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17).unwrap();
    let timestamps = fixture_timestamps();
    let fixture = EventArrayFixture {
        oid,
        timestamps: timestamps.clone().map(|stamp| timestamp_value(&stamp)),
        messages: ["offnormal", "fault", "normal"]
            .map(|message| PropertyValue::CharacterString(message.into())),
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(fixture)).unwrap();
    (db, oid, timestamps)
}

fn timestamp_value(timestamp: &BACnetTimeStamp) -> PropertyValue {
    let mut bytes = BytesMut::new();
    encode_timestamp_choice(&mut bytes, timestamp).unwrap();
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn assert_timestamp_bytes(bytes: &[u8], expected: &[BACnetTimeStamp]) {
    let mut offset = 0;
    for expected in expected {
        let (decoded, next) = decode_timestamp_choice(bytes, offset).unwrap();
        assert_eq!(&decoded, expected);
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(offset, bytes.len());
}

#[test]
fn read_property_preserves_whole_event_timestamp_choice_array() {
    let (db, oid, timestamps) = event_array_db();
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyACK::decode(&ack_buf).unwrap();
    assert_eq!(
        ack.property_identifier,
        PropertyIdentifier::EVENT_TIME_STAMPS
    );
    assert_timestamp_bytes(&ack.property_value, &timestamps);
}

#[test]
fn read_property_preserves_each_indexed_event_timestamp_choice() {
    let (db, oid, timestamps) = event_array_db();
    for (slot, expected) in timestamps.iter().enumerate() {
        let request = ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
            property_array_index: Some(slot as u32 + 1),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property(&db, &buf, &mut ack_buf).unwrap();
        let ack = ReadPropertyACK::decode(&ack_buf).unwrap();
        assert_eq!(ack.property_array_index, Some(slot as u32 + 1));
        assert_timestamp_bytes(&ack.property_value, std::slice::from_ref(expected));
    }
}

#[test]
fn rpm_preserves_event_timestamp_choices_count_and_inline_array_error() {
    let (db, oid, timestamps) = event_array_db();
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
                    property_array_index: Some(0),
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
                    property_array_index: Some(2),
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
                    property_array_index: Some(4),
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                    property_array_index: Some(1),
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&ack_buf).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    assert_eq!(results.len(), 5);

    assert_timestamp_bytes(
        results[0]
            .property_value
            .as_ref()
            .expect("whole timestamp array must succeed"),
        &timestamps,
    );

    let count = results[1]
        .property_value
        .as_ref()
        .expect("timestamp count must succeed");
    let (count, end) = bacnet_encoding::primitives::decode_application_value(count, 0).unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3));
    assert_eq!(end, results[1].property_value.as_ref().unwrap().len());

    assert_timestamp_bytes(
        results[2]
            .property_value
            .as_ref()
            .expect("indexed timestamp must succeed"),
        std::slice::from_ref(&timestamps[1]),
    );

    assert!(results[3].property_value.is_none());
    assert_eq!(
        results[3].error,
        Some((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX))
    );

    let message = results[4]
        .property_value
        .as_ref()
        .expect("message sibling after inline error must succeed");
    let (message, end) = bacnet_encoding::primitives::decode_application_value(message, 0).unwrap();
    assert_eq!(message, PropertyValue::CharacterString("offnormal".into()));
    assert_eq!(end, results[4].property_value.as_ref().unwrap().len());
}
