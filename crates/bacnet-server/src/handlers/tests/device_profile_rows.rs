//! Device profiles retain the Device's dynamic identity and ordinary dispatch.
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

fn profiled() -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = DeviceObject::new(DeviceConfig::default()).unwrap();
    object
        .set_profile(ObjectProfile {
            tags: Some(vec![
                BACnetNameValue::semantic("exhaust"),
                BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            ]),
            profile_location: Some(LOCATION.into()),
            profile_name: Some(NAME.into()),
        })
        .unwrap();
    db_with(Box::new(object))
}

#[test]
fn device_profiles_read_with_exact_wire_bytes() {
    let (db, oid) = profiled();
    assert_eq!(
        read_at(&db, oid, TAGS, None).unwrap(),
        [EXHAUST, FLOOR].concat()
    );
    assert_eq!(
        read_at(&db, oid, PropertyIdentifier::PROFILE_LOCATION, None).unwrap(),
        text(LOCATION)
    );
    assert_eq!(
        read_at(&db, oid, PropertyIdentifier::PROFILE_NAME, None).unwrap(),
        text(NAME)
    );
}

#[test]
fn device_profile_wire_array_operations_and_refusals_preserve_identity() {
    let (mut db, oid) = profiled();
    let identity = read_at(&db, oid, PropertyIdentifier::OBJECT_IDENTIFIER, None).unwrap();
    for (p, index, expected) in [
        (TAGS, None, [EXHAUST, FLOOR].concat()),
        (TAGS, Some(0), vec![0x21, 2]),
        (TAGS, Some(2), FLOOR.to_vec()),
        (PropertyIdentifier::PROFILE_LOCATION, None, text(LOCATION)),
        (PropertyIdentifier::PROFILE_NAME, None, text(NAME)),
    ] {
        assert_eq!(read_output_at(&db, oid, p, index).unwrap(), expected);
    }
    let list = read_at(&db, oid, PropertyIdentifier::PROPERTY_LIST, None).unwrap();
    let mut offset = 0;
    let mut properties = Vec::new();
    while offset < list.len() {
        let (PropertyValue::Enumerated(property), end) =
            bacnet_encoding::primitives::decode_application_value(&list, offset).unwrap()
        else {
            panic!("property identifier");
        };
        properties.push(property);
        offset = end;
    }
    assert!(!properties.contains(&PropertyIdentifier::PROPERTY_LIST.to_raw()));
    assert!(!properties.contains(&PropertyIdentifier::OBJECT_IDENTIFIER.to_raw()));
    for p in [
        TAGS,
        PropertyIdentifier::PROFILE_LOCATION,
        PropertyIdentifier::PROFILE_NAME,
    ] {
        assert!(properties.contains(&p.to_raw()));
    }
    assert_eq!(
        read_output_at(&db, oid, PropertyIdentifier::PROPERTY_LIST, Some(0)).unwrap(),
        [0x21, properties.len() as u8]
    );
    for (index, p) in properties.iter().enumerate() {
        let bytes = read_output_at(
            &db,
            oid,
            PropertyIdentifier::PROPERTY_LIST,
            Some(index as u32 + 1),
        )
        .unwrap();
        let (value, end) =
            bacnet_encoding::primitives::decode_application_value(&bytes, 0).unwrap();
        assert_eq!(value, PropertyValue::Enumerated(*p));
        assert_eq!(end, bytes.len());
    }
    for p in [
        PropertyIdentifier::PROFILE_LOCATION,
        PropertyIdentifier::PROFILE_NAME,
    ] {
        assert_refused(
            read_at(&db, oid, p, Some(1)).map(|_| ()),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
        assert_refused(
            write_at(&mut db, oid, p, None, &text("555-other")),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    for (index, bytes, code) in [
        (Some(3), EXHAUST.to_vec(), ErrorCode::INVALID_ARRAY_INDEX),
        (
            None,
            vec![0x44, 0x3f, 0x80, 0, 0],
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (None, vec![0x0a, 0, b';'], ErrorCode::VALUE_OUT_OF_RANGE),
        (
            None,
            vec![0x0a, 0, b'd', 0xa4, 126, 10, 8, 4, 0xb4, 12, 0, 0, 0],
            ErrorCode::INVALID_DATA_ENCODING,
        ),
    ] {
        assert_refused(write_at(&mut db, oid, TAGS, index, &bytes), code);
        assert_eq!(
            read_at(&db, oid, TAGS, None).unwrap(),
            [EXHAUST, FLOOR].concat()
        );
    }
    write_at(&mut db, oid, TAGS, None, &FLOOR).unwrap();
    write_at(&mut db, oid, TAGS, Some(0), &[0x21, 2]).unwrap();
    assert_eq!(read_output_at(&db, oid, TAGS, Some(2)).unwrap(), [0x09, 0]);
    write_at(&mut db, oid, TAGS, Some(2), &EXHAUST).unwrap();
    assert_eq!(
        read_output_at(&db, oid, TAGS, None).unwrap(),
        [FLOOR, EXHAUST].concat()
    );
    write_at(&mut db, oid, TAGS, None, &[]).unwrap();
    assert_eq!(
        read_output_at(&db, oid, TAGS, None).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(read_output_at(&db, oid, TAGS, Some(0)).unwrap(), [0x21, 0]);
    assert_eq!(
        read_at(&db, oid, PropertyIdentifier::OBJECT_IDENTIFIER, None).unwrap(),
        identity
    );
    let (mut db, oid) = db_with(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ));
    for p in [
        TAGS,
        PropertyIdentifier::PROFILE_LOCATION,
        PropertyIdentifier::PROFILE_NAME,
    ] {
        assert_refused(
            read_at(&db, oid, p, None).map(|_| ()),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_refused(
            write_at(&mut db, oid, p, None, &[0]),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
}
