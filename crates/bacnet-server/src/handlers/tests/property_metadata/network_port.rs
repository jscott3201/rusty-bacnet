use super::*;
use bacnet_objects::{network_port::NetworkPortObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn network_port_object(configured: bool) -> NetworkPortObject {
    let mut object = NetworkPortObject::new(7, "NP-7", 0).unwrap();
    if configured {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long network port label".repeat(100)),
                None,
            )
            .unwrap();
        object
            .write_property(
                P::IP_ADDRESS,
                None,
                PropertyValue::OctetString(vec![192, 168, 1, 100]),
                None,
            )
            .unwrap();
        object
            .write_property(
                P::BACNET_IP_UDP_PORT,
                None,
                PropertyValue::Unsigned(47809),
                None,
            )
            .unwrap();
        object
            .write_property(P::NETWORK_NUMBER, None, PropertyValue::Unsigned(5), None)
            .unwrap();
    }
    // Exercise the unconditional write routes so large encodings persist.
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(configured),
            None,
        )
        .unwrap();
    object
}

#[test]
fn rpm_network_port_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::NETWORK_TYPE,
        P::NETWORK_NUMBER,
        P::MAC_ADDRESS,
        P::MAX_APDU_LENGTH_ACCEPTED,
        P::LINK_SPEED,
        P::CHANGES_PENDING,
        P::COMMAND_NP,
        P::IP_ADDRESS,
        P::IP_DEFAULT_GATEWAY,
        P::IP_SUBNET_MASK,
        P::BACNET_IP_UDP_PORT,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::NETWORK_TYPE,
        P::LINK_SPEED,
        P::CHANGES_PENDING,
    ];
    let optional = [
        P::DESCRIPTION,
        P::NETWORK_NUMBER,
        P::MAC_ADDRESS,
        P::MAX_APDU_LENGTH_ACCEPTED,
        P::COMMAND_NP,
        P::IP_ADDRESS,
        P::IP_DEFAULT_GATEWAY,
        P::IP_SUBNET_MASK,
        P::BACNET_IP_UDP_PORT,
    ];
    for configured in [false, true] {
        let object = network_port_object(configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        for (selector, expected) in [
            (P::ALL, all.as_slice()),
            (P::REQUIRED, required.as_slice()),
            (P::OPTIONAL, optional.as_slice()),
            (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
        ] {
            assert_rpm_selector_bytes(&db, oid, selector, expected);
        }
    }
}

#[test]
fn rpm_network_port_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::NETWORK_PORT),
        ObjectSpecifier::Identifier(oid),
    ] {
        let mut db = ObjectDatabase::new();
        let mut request = BytesMut::new();
        CreateObjectRequest {
            object_specifier,
            list_of_initial_values: vec![],
        }
        .encode(&mut request);
        let mut response = BytesMut::new();
        let result = handle_create_object(&mut db, &request, &mut response);
        assert!(matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32));
        assert!(response.is_empty());
        assert!(db.is_empty());
    }
}

#[test]
fn network_port_delete_object_is_denied() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    // NetworkPort models a running node's port and is not deleteable at
    // runtime, mirroring `NetworkPortObject::is_deleteable` so PICS and the
    // runtime DeleteObject handler share one truth source (differs from
    // LoadControl delete-allowed).
    let mut db = ObjectDatabase::new();
    db.add(Box::new(NetworkPortObject::new(7, "NP-7", 0).unwrap()))
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    let result = handle_delete_object(&mut db, &request);
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw() as u32),
        "DeleteObject must reject NETWORK_PORT, got {result:?}"
    );
    assert!(
        db.get(&oid).is_some(),
        "NetworkPort must still be present after a rejected delete"
    );
}
