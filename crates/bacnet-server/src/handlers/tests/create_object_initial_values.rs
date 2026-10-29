//! CreateObject applies each initial value the way WriteProperty applies its
//! value (#1389, #1416): the array index is checked against the new object,
//! the value is decoded whole with the object's list classification, the
//! raw-octet properties reach the object as their octets, and a NULL the
//! property leaves as it is counts as applied. The first initial value that
//! can't be applied is named by its position, and nothing is created
//! (Clause 15.3).

use super::*;
use PropertyIdentifier as P;
use bacnet_services::common::BACnetPropertyValue;

pub(super) fn initial(
    property: P,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> BACnetPropertyValue {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    raw(property, array_index, &bytes, None)
}

pub(super) fn raw(
    property: P,
    array_index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: array_index,
        value: value.to_vec(),
        priority,
    }
}

pub(super) fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

pub(super) fn request(object_type: ObjectType, values: Vec<BACnetPropertyValue>) -> BytesMut {
    let mut request = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(object_type),
        list_of_initial_values: values,
    }
    .encode(&mut request);
    request
}

/// Create an object of `object_type` from `values`: the new identifier, or
/// the refusal.
pub(super) fn create(
    db: &mut ObjectDatabase,
    object_type: ObjectType,
    values: Vec<BACnetPropertyValue>,
) -> Result<ObjectIdentifier, Error> {
    let mut ack = BytesMut::new();
    handle_create_object(db, &request(object_type, values), &mut ack)?;
    match bacnet_encoding::primitives::decode_application_value(&ack, 0)
        .unwrap()
        .0
    {
        PropertyValue::ObjectIdentifier(oid) => Ok(oid),
        other => panic!("expected the new identifier, got {other:?}"),
    }
}

pub(super) fn read(
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

fn unsigned_list(values: &[u64]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .copied()
            .map(PropertyValue::Unsigned)
            .collect(),
    )
}

#[test]
fn alarm_values_initial_value_is_a_list_at_every_length() {
    let mut db = make_db_with_device_and_ai();
    for object_type in [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE] {
        for values in [&[][..], &[2], &[1, 2]] {
            let list = unsigned_list(values);
            let oid = create(
                &mut db,
                object_type,
                vec![initial(P::ALARM_VALUES, None, &list)],
            )
            .unwrap_or_else(|error| panic!("{object_type:?} {values:?}: {error:?}"));
            assert_eq!(
                read(&db, oid, P::ALARM_VALUES, None),
                list,
                "{object_type:?} {values:?}"
            );
        }
    }
}

#[test]
fn a_scalar_initial_value_with_a_trailing_element_is_refused() {
    let mut db = make_db_with_device_and_ai();
    let before = db.len();
    let two_strings = PropertyValue::List(vec![text("kept"), text("dropped")]);
    // The second element used to be dropped and the object created with the
    // first; the whole value now reaches the scalar property and is refused.
    assert_eq!(
        list_refusal(
            create(
                &mut db,
                ObjectType::ANALOG_INPUT,
                vec![
                    initial(P::OBJECT_NAME, None, &text("AI-new")),
                    initial(P::DESCRIPTION, None, &two_strings),
                ],
            )
            .map(|_| ())
        ),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2)
    );
    assert_eq!(db.len(), before);
    let other = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 9).unwrap();
    assert!(db.check_name_available(&other, "AI-new").is_ok());
}

#[test]
fn an_array_initial_value_writes_its_element() {
    let mut db = make_db_with_device_and_ai();
    let oid = create(
        &mut db,
        ObjectType::MULTI_STATE_VALUE,
        vec![initial(P::STATE_TEXT, Some(2), &text("High"))],
    )
    .unwrap();
    assert_eq!(read(&db, oid, P::STATE_TEXT, Some(2)), text("High"));
    assert_eq!(read(&db, oid, P::STATE_TEXT, Some(1)), text("State 1"));
}

#[test]
fn a_raw_octet_initial_value_reaches_the_object_whole() {
    // Value_Source is one of the properties the decoder hands over as its
    // raw octets. The command's owner corrects the source it just wrote to
    // none (context tag 0, no contents).
    let mut db = make_db_with_device_and_ai();
    let mut real = BytesMut::new();
    encode_property_value(&mut real, &PropertyValue::Real(2.0)).unwrap();
    let mut target = None;
    handle_create_object_observed(
        &mut db,
        &request(
            ObjectType::ANALOG_OUTPUT,
            vec![
                raw(P::PRESENT_VALUE, None, &real, Some(8)),
                raw(P::VALUE_SOURCE, None, &[0x08], Some(8)),
            ],
        ),
        &mut BytesMut::new(),
        &mut target,
        Some(&crate::command_source::test_origin()),
    )
    .unwrap();
    let oid = target.unwrap();
    assert_eq!(
        read(&db, oid, P::PRESENT_VALUE, None),
        PropertyValue::Real(2.0)
    );
    assert_eq!(
        read(&db, oid, P::VALUE_SOURCE, None),
        PropertyValue::ApplicationData(vec![0x08])
    );
}

#[test]
fn a_null_initial_value_leaves_a_noncommandable_property_as_it_is() {
    let mut db = make_db_with_device_and_ai();
    let null = PropertyValue::Null;
    let after = text("after the NULLs");
    for (object_type, nulls) in [
        (
            ObjectType::ANALOG_INPUT,
            vec![(P::OBJECT_NAME, None), (P::COV_INCREMENT, None)],
        ),
        (
            ObjectType::MULTI_STATE_VALUE,
            vec![(P::ALARM_VALUES, None), (P::STATE_TEXT, Some(2))],
        ),
    ] {
        let plain = create(&mut db, object_type, vec![]).unwrap();
        let mut values: Vec<_> = nulls
            .iter()
            .map(|&(property, index)| initial(property, index, &null))
            .collect();
        values.push(initial(P::DESCRIPTION, None, &after));
        let oid = create(&mut db, object_type, values)
            .unwrap_or_else(|error| panic!("{object_type:?}: {error:?}"));
        // Each NULL left its property at the value a new object starts
        // with, and the initial value after them was applied.
        for &(property, index) in &nulls {
            let expected = if property == P::OBJECT_NAME {
                text(&format!("{object_type}-{}", oid.instance_number()))
            } else {
                read(&db, plain, property, index)
            };
            assert_eq!(
                read(&db, oid, property, index),
                expected,
                "{object_type:?} {property:?}"
            );
        }
        assert_eq!(read(&db, oid, P::DESCRIPTION, None), after);
    }
}

#[test]
fn a_null_initial_value_still_relinquishes_a_commandable_present_value() {
    let mut db = make_db_with_device_and_ai();
    let mut real = BytesMut::new();
    encode_property_value(&mut real, &PropertyValue::Real(2.0)).unwrap();
    let mut target = None;
    handle_create_object_observed(
        &mut db,
        &request(
            ObjectType::ANALOG_OUTPUT,
            vec![
                raw(P::PRESENT_VALUE, None, &real, Some(8)),
                raw(P::PRESENT_VALUE, None, &[0x00], Some(8)),
            ],
        ),
        &mut BytesMut::new(),
        &mut target,
        Some(&crate::command_source::test_origin()),
    )
    .unwrap();
    let oid = target.unwrap();
    assert_eq!(
        read(&db, oid, P::PRIORITY_ARRAY, Some(8)),
        PropertyValue::Null
    );
    assert_eq!(
        read(&db, oid, P::PRESENT_VALUE, None),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn the_first_initial_value_that_fails_is_named_and_nothing_is_created() {
    let mut db = make_db_with_device_and_ai();
    let before = db.len();
    let name = || initial(P::OBJECT_NAME, None, &text("taken-if-kept"));
    let description = || initial(P::DESCRIPTION, None, &text("ok"));
    let null = PropertyValue::Null;
    let alarm_with_a_real =
        PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Real(1.0)]);
    for (what, object_type, values, expected, malformed) in [
        (
            "an index on a property that isn't an array",
            ObjectType::ANALOG_INPUT,
            vec![name(), initial(P::DESCRIPTION, Some(1), &text("x"))],
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY, 2),
            false,
        ),
        (
            "an index past the array",
            ObjectType::MULTI_STATE_VALUE,
            vec![
                description(),
                name(),
                initial(P::STATE_TEXT, Some(9), &text("x")),
            ],
            (ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX, 3),
            false,
        ),
        (
            "a NULL to a read-only property",
            ObjectType::ANALOG_INPUT,
            vec![name(), description(), initial(P::STATUS_FLAGS, None, &null)],
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED, 3),
            false,
        ),
        (
            "a list with an element of another datatype",
            ObjectType::MULTI_STATE_VALUE,
            vec![initial(P::ALARM_VALUES, None, &alarm_with_a_real), name()],
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 1),
            false,
        ),
        (
            "no octets on a scalar property",
            ObjectType::ANALOG_INPUT,
            vec![name(), raw(P::DESCRIPTION, None, &[], None)],
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING, 2),
            true,
        ),
        (
            "a trailing element that doesn't decode",
            ObjectType::MULTI_STATE_VALUE,
            vec![
                name(),
                raw(P::ALARM_VALUES, None, &[0x21, 0x01, 0xD1, 0x00], None),
            ],
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING, 2),
            true,
        ),
    ] {
        let mut target = None;
        let refusal = handle_create_object_observed(
            &mut db,
            &request(object_type, values),
            &mut BytesMut::new(),
            &mut target,
            None,
        )
        .unwrap_err();
        // Octets that don't decode make the request invalid, which isn't
        // audited; any other refusal is an attempt that failed.
        assert_eq!(
            matches!(refusal, CreateObjectRefusal::Malformed(_)),
            malformed,
            "{what}: {refusal:?}"
        );
        assert_eq!(list_refusal(Err(refusal.into_error())), expected, "{what}");
        assert_eq!(db.len(), before, "{what}");
        assert!(db.get(&target.unwrap()).is_none(), "{what}");
        let other = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 9).unwrap();
        assert!(
            db.check_name_available(&other, "taken-if-kept").is_ok(),
            "{what}"
        );
    }
}
