use std::borrow::Cow;

use super::*;
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

#[test]
fn all_nine_builtin_intrinsic_families_implement_atomic_commit() {
    let mut objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(AnalogInputObject::new(1, "AI", 0).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO", 0).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", 0).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
    ];

    for object in &mut objects {
        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: BACnetTimeStamp::SequenceNumber(73),
                message_text: None,
            })
            .unwrap_or_else(|error| {
                panic!(
                    "{} must implement the shared commit hook: {error:?}",
                    object.object_name()
                )
            });

        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
            "{} Event_State",
            object.object_name()
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x60],
            },
            "{} Acked_Transitions",
            object.object_name()
        );
    }
}

#[test]
fn event_sequence_wraps_and_is_database_local() {
    let mut first = ObjectDatabase::new();
    let mut second = ObjectDatabase::new();

    assert_eq!(first.next_event_sequence_number(), 0);
    assert_eq!(first.next_event_sequence_number(), 1);
    assert_eq!(second.next_event_sequence_number(), 0);

    first.event_sequence_number = u16::MAX;
    assert_eq!(first.next_event_sequence_number(), u16::MAX);
    assert_eq!(first.next_event_sequence_number(), 0);
    assert_eq!(second.next_event_sequence_number(), 1);
}

#[test]
fn event_sequence_reservation_is_non_consuming_until_confirmed() {
    let mut db = ObjectDatabase::new();

    let reservation = db.reserve_event_sequence_number();
    assert_eq!(reservation.number(), 0);
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    assert!(db.confirm_event_sequence_number(reservation));
    assert_eq!(db.reserve_event_sequence_number().number(), 1);
}

/// Minimal test object.
struct TestObject {
    oid: ObjectIdentifier,
    name: String,
}

impl BACnetObject for TestObject {
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
        if property == PropertyIdentifier::OBJECT_NAME {
            Ok(PropertyValue::CharacterString(self.name.clone()))
        } else {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
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

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[PropertyIdentifier::OBJECT_NAME])
    }
}

fn make_test_object(instance: u32) -> Box<dyn BACnetObject> {
    Box::new(TestObject {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        name: format!("AI-{instance}"),
    })
}

fn make_test_object_typed(
    object_type: ObjectType,
    instance: u32,
    name: &str,
) -> Box<dyn BACnetObject> {
    Box::new(TestObject {
        oid: ObjectIdentifier::new(object_type, instance).unwrap(),
        name: name.to_string(),
    })
}

#[test]
fn add_and_get() {
    let mut db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(make_test_object(1)).unwrap();
    assert_eq!(db.len(), 1);

    let obj = db.get(&oid).unwrap();
    assert_eq!(obj.object_name(), "AI-1");
}

#[test]
fn get_nonexistent_returns_none() {
    let db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
    assert!(db.get(&oid).is_none());
}

#[test]
fn read_property_via_database() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object(1)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let obj = db.get(&oid).unwrap();
    let val = obj
        .read_property(PropertyIdentifier::OBJECT_NAME, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("AI-1".into()));
}

#[test]
fn remove_object() {
    let mut db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(make_test_object(1)).unwrap();
    assert_eq!(db.len(), 1);
    let removed = db.remove(&oid);
    assert!(removed.is_some());
    assert_eq!(db.len(), 0);
}

#[test]
fn list_objects() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object(1)).unwrap();
    db.add(make_test_object(2)).unwrap();
    let oids = db.list_objects();
    assert_eq!(oids.len(), 2);
}

#[test]
fn find_by_type_returns_matching_objects() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
        .unwrap();
    db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 2, "AI-2"))
        .unwrap();
    db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "BI-1"))
        .unwrap();
    db.add(make_test_object_typed(ObjectType::ANALOG_OUTPUT, 1, "AO-1"))
        .unwrap();

    let ai_oids = db.find_by_type(ObjectType::ANALOG_INPUT);
    assert_eq!(ai_oids.len(), 2);
    for oid in &ai_oids {
        assert_eq!(oid.object_type(), ObjectType::ANALOG_INPUT);
    }

    let bi_oids = db.find_by_type(ObjectType::BINARY_INPUT);
    assert_eq!(bi_oids.len(), 1);
    assert_eq!(bi_oids[0].object_type(), ObjectType::BINARY_INPUT);
    assert_eq!(bi_oids[0].instance_number(), 1);

    let ao_oids = db.find_by_type(ObjectType::ANALOG_OUTPUT);
    assert_eq!(ao_oids.len(), 1);
}

#[test]
fn find_by_type_returns_empty_for_no_matches() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
        .unwrap();

    let results = db.find_by_type(ObjectType::BINARY_VALUE);
    assert!(results.is_empty());
}

#[test]
fn find_by_type_on_empty_database() {
    let db = ObjectDatabase::new();
    let results = db.find_by_type(ObjectType::ANALOG_INPUT);
    assert!(results.is_empty());
}

#[test]
fn iter_objects_yields_all_entries() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "AI-1"))
        .unwrap();
    db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "BI-1"))
        .unwrap();

    let items: Vec<_> = db.iter_objects().collect();
    assert_eq!(items.len(), 2);

    // Verify we can access object data without a second lookup
    for (oid, obj) in &items {
        assert_eq!(oid.object_type(), obj.object_identifier().object_type());
        assert!(!obj.object_name().is_empty());
    }
}

#[test]
fn iter_objects_on_empty_database() {
    let db = ObjectDatabase::new();
    assert_eq!(db.iter_objects().count(), 0);
}

#[test]
fn duplicate_name_rejected() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        1,
        "Sensor",
    ))
    .unwrap();
    // Different OID, same name → must fail
    let result = db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        2,
        "Sensor",
    ));
    assert!(result.is_err());
    assert_eq!(db.len(), 1); // original still there
}

#[test]
fn replace_same_oid_allowed() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        1,
        "Sensor",
    ))
    .unwrap();
    // Same OID, same or different name → allowed (replacement)
    db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        1,
        "Sensor-v2",
    ))
    .unwrap();
    assert_eq!(db.len(), 1);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(db.get(&oid).unwrap().object_name(), "Sensor-v2");
}

#[test]
fn find_by_name_works() {
    let mut db = ObjectDatabase::new();
    db.add(make_test_object_typed(ObjectType::ANALOG_INPUT, 1, "Temp"))
        .unwrap();
    db.add(make_test_object_typed(ObjectType::BINARY_INPUT, 1, "Alarm"))
        .unwrap();

    let obj = db.find_by_name("Temp").unwrap();
    assert_eq!(obj.object_identifier().instance_number(), 1);
    assert_eq!(
        obj.object_identifier().object_type(),
        ObjectType::ANALOG_INPUT
    );

    assert!(db.find_by_name("NonExistent").is_none());
}

#[test]
fn remove_frees_name() {
    let mut db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        1,
        "Sensor",
    ))
    .unwrap();
    db.remove(&oid);
    // Name should now be available for a different object
    db.add(make_test_object_typed(
        ObjectType::ANALOG_INPUT,
        2,
        "Sensor",
    ))
    .unwrap();
    assert_eq!(db.len(), 1);
}
