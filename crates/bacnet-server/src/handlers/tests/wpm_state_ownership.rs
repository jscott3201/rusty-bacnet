use super::*;
use bacnet_objects::{
    access_control::AccessDoorObject, file::FileObject, network_port::NetworkPortObject,
};

fn write(
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: encode_value(&value),
        priority,
    }
}

fn prefix_failure(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    mut prefix: Vec<BACnetPropertyValue>,
    suffix: BACnetPropertyValue,
) {
    prefix.push(write(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyValue::ObjectIdentifier(oid),
        None,
    ));
    prefix.push(suffix);
    let WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    } = detailed(db, &encode_request(oid, prefix))
    else {
        panic!("expected WPM failure")
    };
    assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_reference(
        &first_failed_write_attempt,
        oid,
        PropertyIdentifier::OBJECT_IDENTIFIER,
        None,
    );
    assert_eq!(committed_oids, vec![oid]);
}

fn read(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

#[test]
fn wpm_file_resize_and_network_configuration_keep_committed_state() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "File", "binary").unwrap();
    file.set_data(vec![1, 2, 3]);
    let file_oid = file.object_identifier();
    db.add(Box::new(file)).unwrap();
    prefix_failure(
        &mut db,
        file_oid,
        vec![write(
            PropertyIdentifier::FILE_SIZE,
            PropertyValue::Unsigned(2),
            None,
        )],
        write(
            PropertyIdentifier::FILE_SIZE,
            PropertyValue::Unsigned(1),
            None,
        ),
    );
    assert_eq!(
        read(&db, file_oid, PropertyIdentifier::FILE_SIZE, None),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        db.get(&file_oid)
            .unwrap()
            .file_storage_internal()
            .unwrap()
            .read_stream(0, 10)
            .unwrap()
            .data,
        vec![1, 2]
    );
    assert_eq!(
        read(&db, file_oid, PropertyIdentifier::ARCHIVE, None),
        PropertyValue::Boolean(false)
    );

    let mut port = NetworkPortObject::new(1, "Port", 0).unwrap();
    let port_oid = port.object_identifier();
    let before = port
        .read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
        .unwrap();
    assert!(port
        .write_property(
            PropertyIdentifier::BACNET_IP_UDP_PORT,
            None,
            PropertyValue::Unsigned(65536),
            None
        )
        .is_err());
    assert_eq!(
        port.read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
            .unwrap(),
        before
    );
    assert_eq!(
        port.read_property(PropertyIdentifier::CHANGES_PENDING, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    db.add(Box::new(port)).unwrap();
    prefix_failure(
        &mut db,
        port_oid,
        vec![write(
            PropertyIdentifier::IP_ADDRESS,
            PropertyValue::OctetString(vec![10, 0, 0, 1]),
            None,
        )],
        write(
            PropertyIdentifier::IP_ADDRESS,
            PropertyValue::OctetString(vec![10, 0, 0, 2]),
            None,
        ),
    );
    assert_eq!(
        read(&db, port_oid, PropertyIdentifier::IP_ADDRESS, None),
        PropertyValue::OctetString(vec![10, 0, 0, 1])
    );
    assert_eq!(
        read(&db, port_oid, PropertyIdentifier::CHANGES_PENDING, None),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn wpm_command_priority_and_fallback_remain_owned_by_committed_prefix() {
    let objects: Vec<(
        Box<dyn BACnetObject>,
        PropertyValue,
        PropertyValue,
        PropertyValue,
    )> = vec![
        (
            Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(1),
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(1),
        ),
        (
            Box::new(AccessDoorObject::new(1, "Door").unwrap()),
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(2),
            PropertyValue::Enumerated(0),
        ),
    ];
    for (object, fallback, commanded, suffix) in objects {
        let mut db = ObjectDatabase::new();
        let oid = object.object_identifier();
        db.add(object).unwrap();
        prefix_failure(
            &mut db,
            oid,
            vec![
                write(
                    PropertyIdentifier::RELINQUISH_DEFAULT,
                    fallback.clone(),
                    None,
                ),
                write(
                    PropertyIdentifier::PRESENT_VALUE,
                    commanded.clone(),
                    Some(8),
                ),
            ],
            write(PropertyIdentifier::PRESENT_VALUE, suffix, Some(1)),
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::PRESENT_VALUE, None),
            commanded
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)),
            commanded
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(1)),
            PropertyValue::Null
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::RELINQUISH_DEFAULT, None),
            fallback
        );
        db.get_mut(&oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Null,
                Some(8),
            )
            .unwrap();
        assert_eq!(
            read(&db, oid, PropertyIdentifier::PRESENT_VALUE, None),
            fallback
        );
    }
}
