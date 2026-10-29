//! The name CreateObject gives a new object before its initial values apply
//! (#1437). It is `<TYPE>-<instance>`, or the first free `<TYPE>-<instance>
//! (n)` when another object holds that, so a client that renamed an object
//! to the next generated name doesn't make the create fail.

use super::create_object_initial_values::{create, initial, read, text};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::binary::BinaryInputObject;

fn ai_1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// The device and AI-1, the AI renamed to the name the next Binary Value
/// would be given.
fn db_with_the_default_name_taken() -> ObjectDatabase {
    let mut db = make_db_with_device_and_ai();
    let mut octets = BytesMut::new();
    encode_property_value(&mut octets, &text("BINARY_VALUE-1")).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ai_1(),
        property_identifier: P::OBJECT_NAME,
        property_array_index: None,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(&mut db, &request).unwrap();
    db
}

fn name_of(db: &ObjectDatabase, oid: ObjectIdentifier) -> PropertyValue {
    read(db, oid, P::OBJECT_NAME, None)
}

#[test]
fn a_taken_default_name_gives_way_to_the_requested_one() {
    let mut db = db_with_the_default_name_taken();
    let oid = create(
        &mut db,
        ObjectType::BINARY_VALUE,
        vec![initial(P::OBJECT_NAME, None, &text("Fan"))],
    )
    .unwrap();
    assert_eq!(oid.instance_number(), 1);
    assert_eq!(name_of(&db, oid), text("Fan"));
    assert_eq!(db.find_by_name("Fan").unwrap().object_identifier(), oid);
    // The renamed object keeps its name, and the stand-in is gone.
    assert_eq!(name_of(&db, ai_1()), text("BINARY_VALUE-1"));
    assert!(db.find_by_name("BINARY_VALUE-1 (2)").is_none());
}

#[test]
fn a_taken_default_name_takes_the_first_free_suffix() {
    let mut db = db_with_the_default_name_taken();
    let first = create(&mut db, ObjectType::BINARY_VALUE, vec![]).unwrap();
    assert_eq!(name_of(&db, first), text("BINARY_VALUE-1 (2)"));
    // The next instance's own name is free again.
    let second = create(&mut db, ObjectType::BINARY_VALUE, vec![]).unwrap();
    assert_eq!(name_of(&db, second), text("BINARY_VALUE-2"));

    // With the first suffix taken too, the count goes on.
    let mut db = db_with_the_default_name_taken();
    db.add(Box::new(
        BinaryInputObject::new(1, "BINARY_VALUE-1 (2)").unwrap(),
    ))
    .unwrap();
    let mut ack = BytesMut::new();
    let mut request = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(
            ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap(),
        ),
        list_of_initial_values: vec![initial(P::DESCRIPTION, None, &text("d"))],
    }
    .encode(&mut request);
    handle_create_object(&mut db, &request, &mut ack).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    assert_eq!(name_of(&db, oid), text("BINARY_VALUE-1 (3)"));
}

#[test]
fn a_requested_name_that_is_taken_is_still_refused_at_its_position() {
    let mut db = db_with_the_default_name_taken();
    let before = db.len();
    assert_eq!(
        list_refusal(
            create(
                &mut db,
                ObjectType::BINARY_VALUE,
                vec![
                    initial(P::DESCRIPTION, None, &text("d")),
                    initial(P::OBJECT_NAME, None, &text("BINARY_VALUE-1")),
                ],
            )
            .map(|_| ())
        ),
        (ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME, 2)
    );
    // Nothing is created and the stand-in name isn't left behind.
    assert_eq!(db.len(), before);
    assert!(db.find_by_name("BINARY_VALUE-1 (2)").is_none());
    assert_eq!(name_of(&db, ai_1()), text("BINARY_VALUE-1"));
}
