//! Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair written as
//! a WriteProperty arrives (#1329): the reference's context-tagged members
//! through the generic decoder, the inhibit refused while a reference is
//! set, and the message texts by array index, whose size, fixed by the
//! tables, takes no write at index 0.

use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn switch(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap()
}

fn application(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

/// A reference's members, context-tagged, as the request carries them.
fn reference_to(target: ObjectIdentifier) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_object_property_reference(
        &mut bytes,
        &BACnetObjectPropertyReference::new(target, P::PRESENT_VALUE.to_raw()),
    );
    bytes.to_vec()
}

fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: Vec<u8>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn read(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

fn refused(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
        "expected {expected:?}, got {result:?}"
    );
}

fn db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut active = BinaryValueObject::new(1, "BV-1").unwrap();
    active
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    db.add(Box::new(active)).unwrap();
    db
}

#[test]
fn a_reference_written_over_the_wire_makes_the_inhibit_follow_it() {
    let mut db = db();
    wp(
        &mut db,
        ai(),
        P::EVENT_ALGORITHM_INHIBIT_REF,
        None,
        reference_to(switch(1)),
    )
    .unwrap();
    assert_eq!(
        read(&db, ai(), P::EVENT_ALGORITHM_INHIBIT_REF, None),
        PropertyValue::ApplicationData(reference_to(switch(1)))
    );
    // The Binary Value is ACTIVE, so following it inhibits.
    assert!(db.follow_event_algorithm_inhibit(&ai()));
    assert_eq!(
        read(&db, ai(), P::EVENT_ALGORITHM_INHIBIT, None),
        PropertyValue::Boolean(true)
    );
    // While the reference is set, the inhibit takes no write.
    let off = application(&PropertyValue::Boolean(false));
    refused(
        wp(&mut db, ai(), P::EVENT_ALGORITHM_INHIBIT, None, off.clone()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // Cleared with the reserved instance, the reference leaves the inhibit
    // FALSE and writable again.
    wp(
        &mut db,
        ai(),
        P::EVENT_ALGORITHM_INHIBIT_REF,
        None,
        reference_to(switch(4_194_303)),
    )
    .unwrap();
    assert_eq!(
        read(&db, ai(), P::EVENT_ALGORITHM_INHIBIT, None),
        PropertyValue::Boolean(false)
    );
    wp(&mut db, ai(), P::EVENT_ALGORITHM_INHIBIT, None, off).unwrap();
}

#[test]
fn message_texts_config_is_written_by_index_and_no_size_is_written() {
    let mut db = db();
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    wp(
        &mut db,
        ai(),
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        Some(2),
        application(&text("Sensor broken")),
    )
    .unwrap();
    assert_eq!(
        read(&db, ai(), P::EVENT_MESSAGE_TEXTS_CONFIG, None),
        PropertyValue::List(vec![text(""), text("Sensor broken"), text("")])
    );
    refused(
        wp(
            &mut db,
            ai(),
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(4),
            application(&text("x")),
        ),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    // The message texts are always three, so their size takes no write
    // (Clause 12.1.5.1); State_Text's does (state_text_count.rs).
    let size = application(&PropertyValue::Unsigned(2));
    refused(
        wp(&mut db, ai(), P::EVENT_MESSAGE_TEXTS_CONFIG, Some(0), size),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read(&db, ai(), P::EVENT_MESSAGE_TEXTS_CONFIG, Some(0)),
        PropertyValue::Unsigned(3)
    );
}
