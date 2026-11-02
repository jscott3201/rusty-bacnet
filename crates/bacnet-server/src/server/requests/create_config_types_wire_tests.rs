//! The configuration objects CreateObject builds since #2208 (Calendar,
//! Schedule, Notification Class, Trend Log, Event Enrollment and Structured
//! View), over the wire: each is created bare with no initial values, takes
//! the values clients give it (creation-only properties among them), refuses
//! a bad one naming its position (Clause 15.3.1.3) and leaves nothing
//! behind, and is deleted cleanly.

use super::create_object_types_wire_tests::{created, request, send};
use super::mutation_tests::{Fixture, oid, value};
use super::*;
use bacnet_encoding::constructed::{
    encode_device_object_property_reference, encode_device_object_reference, encode_event_parameter,
};
use bacnet_services::object_mgmt::{DeleteObjectRequest, ObjectSpecifier};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetEventParameter,
};

use PropertyIdentifier as P;

/// The mutation fixture with a Device clock, which a Trend Log's reference
/// write needs for the purge it records.
async fn clocked() -> Fixture {
    let fixture = Fixture::new(None);
    fixture
        .db
        .write()
        .await
        .set_clock_reader(Some(crate::schedule::tests::SettableClock::at(
            2026, 10, 5, 9, 0,
        )));
    fixture
}

/// The six types, in the order the table lists them.
pub(super) const CONFIG_TYPES: [ObjectType; 6] = [
    ObjectType::CALENDAR,
    ObjectType::SCHEDULE,
    ObjectType::NOTIFICATION_CLASS,
    ObjectType::TREND_LOG,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::STRUCTURED_VIEW,
];

/// date `[0]`: Monday 5 October 2026.
pub(super) const OCTOBER_5: &[u8] = &[0x0C, 126, 10, 5, 1];

/// The Present_Value of `object` as a local reference.
pub(super) fn reference(object: ObjectIdentifier) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_device_object_property_reference(
        &mut buf,
        &BACnetDeviceObjectPropertyReference::new_local(object, P::PRESENT_VALUE.to_raw()),
    );
    buf.to_vec()
}

fn read(
    db: &ObjectDatabase,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    db.get(&object)
        .expect("created")
        .read_property(property, None)
        .unwrap()
}

fn unsigned(values: &[u64]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .copied()
            .map(PropertyValue::Unsigned)
            .collect(),
    )
}

/// The CreateObject-Error for invoke ID 5: PROPERTY / `code` at `element`.
#[track_caller]
fn assert_refused(response: &[u8], code: ErrorCode, element: u8) {
    let create = ConfirmedServiceChoice::CREATE_OBJECT.to_raw();
    assert_eq!(
        response,
        [
            0x50,
            5,
            create,
            0x0E,
            0x91,
            ErrorClass::PROPERTY.to_raw() as u8,
            0x91,
            code.to_raw() as u8,
            0x0F,
            0x19,
            element,
        ]
    );
}

#[tokio::test]
async fn each_configuration_type_is_created_bare_without_initial_values() {
    let fixture = Fixture::new(None);
    for kind in CONFIG_TYPES {
        let object = created(&send(&fixture, request(ObjectSpecifier::Type(kind), vec![])).await);
        assert_eq!(object, oid(kind, 1), "{kind:?}");
    }
    let db = fixture.db.read().await;
    let schedule = oid(ObjectType::SCHEDULE, 1);
    assert_eq!(
        read(&db, schedule, P::SCHEDULE_DEFAULT),
        PropertyValue::Null
    );
    assert_eq!(
        read(&db, schedule, P::LIST_OF_OBJECT_PROPERTY_REFERENCES),
        PropertyValue::ApplicationData(vec![])
    );
    let class = oid(ObjectType::NOTIFICATION_CLASS, 1);
    assert_eq!(read(&db, class, P::PRIORITY), unsigned(&[255, 255, 255]));
    let log = oid(ObjectType::TREND_LOG, 1);
    assert_eq!(read(&db, log, P::BUFFER_SIZE), PropertyValue::Unsigned(100));
    let enrollment = oid(ObjectType::EVENT_ENROLLMENT, 1);
    assert_eq!(
        read(&db, enrollment, P::EVENT_TYPE),
        PropertyValue::Enumerated(bacnet_types::enums::EventType::NONE.to_raw())
    );
}

/// Each type's initial values, as a client configuring it would send them.
fn configured(kind: ObjectType) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    let bv1 = oid(ObjectType::BINARY_VALUE, 1);
    let name = (
        P::OBJECT_NAME,
        value(PropertyValue::CharacterString(format!("configured {kind}"))),
    );
    let mut values = match kind {
        k if k == ObjectType::CALENDAR => vec![(P::DATE_LIST, OCTOBER_5.to_vec())],
        k if k == ObjectType::SCHEDULE => vec![
            (P::SCHEDULE_DEFAULT, value(PropertyValue::Enumerated(1))),
            (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, reference(bv1)),
            (P::PRIORITY_FOR_WRITING, value(PropertyValue::Unsigned(12))),
        ],
        k if k == ObjectType::NOTIFICATION_CLASS => vec![
            (P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(9))),
            (P::PRIORITY, value(unsigned(&[3, 4, 5]))),
            (
                P::ACK_REQUIRED,
                value(PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0xA0],
                }),
            ),
        ],
        k if k == ObjectType::TREND_LOG => vec![
            (P::BUFFER_SIZE, value(PropertyValue::Unsigned(50))),
            (P::LOG_DEVICE_OBJECT_PROPERTY, reference(bv1)),
            (P::LOG_INTERVAL, value(PropertyValue::Unsigned(100))),
        ],
        k if k == ObjectType::EVENT_ENROLLMENT => {
            let mut parameters = BytesMut::new();
            encode_event_parameter(
                &mut parameters,
                &BACnetEventParameter::OutOfRange {
                    time_delay: 0,
                    low_limit: 10.0,
                    high_limit: 30.0,
                    deadband: 1.0,
                },
            )
            .unwrap();
            vec![
                (P::OBJECT_PROPERTY_REFERENCE, reference(bv1)),
                (P::EVENT_PARAMETERS, parameters.to_vec()),
                (P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(9))),
            ]
        }
        _ => {
            let mut subordinates = BytesMut::new();
            for instance in [1, 2] {
                encode_device_object_reference(
                    &mut subordinates,
                    &BACnetDeviceObjectReference {
                        device_identifier: None,
                        object_identifier: oid(ObjectType::BINARY_VALUE, instance),
                    },
                );
            }
            vec![
                (P::NODE_TYPE, value(PropertyValue::Enumerated(6))),
                (P::SUBORDINATE_LIST, subordinates.to_vec()),
                (
                    P::SUBORDINATE_ANNOTATIONS,
                    value(PropertyValue::List(vec![
                        PropertyValue::CharacterString("one".into()),
                        PropertyValue::CharacterString("two".into()),
                    ])),
                ),
            ]
        }
    };
    values.insert(0, name);
    values
}

#[tokio::test]
async fn each_configuration_type_takes_the_initial_values_clients_send() {
    let fixture = clocked().await;
    for kind in CONFIG_TYPES {
        let wanted = oid(kind, 30);
        let response = send(
            &fixture,
            request(ObjectSpecifier::Identifier(wanted), configured(kind)),
        )
        .await;
        assert_eq!(created(&response), wanted, "{kind:?}");
        let db = fixture.db.read().await;
        // The name index follows the name, whichever route set it.
        let named = db.find_by_name(&format!("configured {kind}"));
        assert_eq!(named.map(|object| object.object_identifier()), Some(wanted));
        assert!(db.find_by_name(&format!("{kind}-30")).is_none(), "{kind:?}");
        // Every value reads back as it was sent, decoded the way a
        // WriteProperty of it is.
        for (property, sent) in configured(kind) {
            let object = db.get(&wanted).unwrap();
            let decoded = bacnet_device_core::write::decode_write_property_value(
                property,
                None,
                object.is_list_property(property),
                &sent,
            )
            .unwrap();
            let decoded = bacnet_device_core::write::localize(&db, wanted, property, decoded);
            let served = read(&db, wanted, property);
            match (&decoded, &served) {
                // Constructed values read back as one chunk per element.
                (PropertyValue::ApplicationData(bytes), PropertyValue::List(chunks)) => {
                    let joined: Vec<u8> = chunks
                        .iter()
                        .flat_map(|chunk| match chunk {
                            PropertyValue::ApplicationData(chunk) => chunk.clone(),
                            other => panic!("{kind:?} {property:?}: {other:?}"),
                        })
                        .collect();
                    assert_eq!(&joined, bytes, "{kind:?} {property:?}");
                }
                _ => assert_eq!(served, decoded, "{kind:?} {property:?}"),
            }
        }
    }
    let db = fixture.db.read().await;
    assert_eq!(
        read(&db, oid(ObjectType::EVENT_ENROLLMENT, 30), P::EVENT_TYPE),
        PropertyValue::Enumerated(bacnet_types::enums::EventType::OUT_OF_RANGE.to_raw())
    );
}

#[tokio::test]
async fn a_bad_initial_value_is_refused_at_its_position_and_nothing_is_created() {
    let fixture = Fixture::new(None);
    let mut foreign = BytesMut::new();
    let mut not_a_device =
        BACnetDeviceObjectPropertyReference::new_local(oid(ObjectType::BINARY_VALUE, 1), 85);
    not_a_device.device_identifier = Some(oid(ObjectType::BINARY_VALUE, 2));
    encode_device_object_property_reference(&mut foreign, &not_a_device);
    let cases = [
        // Month 15: 13 and 14 are the odd and even months.
        (
            ObjectType::CALENDAR,
            P::DATE_LIST,
            vec![0x0C, 126, 15, 5, 1],
        ),
        (
            ObjectType::SCHEDULE,
            P::PRIORITY_FOR_WRITING,
            value(PropertyValue::Unsigned(0)),
        ),
        (
            ObjectType::NOTIFICATION_CLASS,
            P::PRIORITY,
            value(unsigned(&[1, 2])),
        ),
        (
            ObjectType::TREND_LOG,
            P::BUFFER_SIZE,
            value(PropertyValue::Unsigned(10_001)),
        ),
        (
            ObjectType::EVENT_ENROLLMENT,
            P::OBJECT_PROPERTY_REFERENCE,
            foreign.to_vec(),
        ),
        (
            ObjectType::STRUCTURED_VIEW,
            P::NODE_TYPE,
            value(PropertyValue::Enumerated(99)),
        ),
    ];
    for (kind, property, bad) in cases {
        let before = fixture.db.read().await.len();
        let name = value(PropertyValue::CharacterString(format!("bad {kind}")));
        let response = send(
            &fixture,
            request(
                ObjectSpecifier::Type(kind),
                vec![(P::OBJECT_NAME, name), (property, bad)],
            ),
        )
        .await;
        assert_refused(&response, ErrorCode::VALUE_OUT_OF_RANGE, 2);
        let db = fixture.db.read().await;
        assert_eq!(db.len(), before, "{kind:?}");
        assert!(db.find_by_type(kind).is_empty(), "{kind:?}");
    }
}

#[tokio::test]
async fn a_created_configuration_object_is_deleted_cleanly() {
    let fixture = clocked().await;
    let before = fixture.db.read().await.len();
    for kind in CONFIG_TYPES {
        let object = created(
            &send(
                &fixture,
                request(ObjectSpecifier::Type(kind), configured(kind)),
            )
            .await,
        );
        let mut bytes = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: object,
        }
        .encode(&mut bytes);
        let delete = ConfirmedServiceChoice::DELETE_OBJECT;
        let npdu = fixture
            .dispatch(delete, bytes.freeze(), 6)
            .await
            .expect("response");
        let response = bacnet_encoding::npdu::decode_npdu(npdu).unwrap().payload;
        assert_eq!(&response[..], &[0x20, 6, delete.to_raw()], "{kind:?}");
        let db = fixture.db.read().await;
        assert!(db.get(&object).is_none(), "{kind:?}");
        assert!(db.find_by_type(kind).is_empty(), "{kind:?}");
        assert!(
            db.find_by_name(&format!("configured {kind}")).is_none(),
            "{kind:?}"
        );
    }
    assert_eq!(fixture.db.read().await.len(), before);
}
