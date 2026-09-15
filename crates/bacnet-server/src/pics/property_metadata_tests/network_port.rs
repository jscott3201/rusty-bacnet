use super::*;
use bacnet_objects::{network_port::NetworkPortObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

#[test]
fn pics_network_port_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, false, false),
        (P::NETWORK_TYPE, false, false),
        (P::NETWORK_NUMBER, true, true),
        (P::MAC_ADDRESS, true, true),
        (P::MAX_APDU_LENGTH_ACCEPTED, true, false),
        (P::LINK_SPEED, false, false),
        (P::CHANGES_PENDING, false, false),
        (P::COMMAND_NP, true, true),
        (P::IP_ADDRESS, true, true),
        (P::IP_DEFAULT_GATEWAY, true, true),
        (P::IP_SUBNET_MASK, true, true),
        (P::BACNET_IP_UDP_PORT, true, true),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
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
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::NETWORK_PORT);
            assert!(!support.createable);
            assert!(!support.deleteable);
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            assert_eq!(
                rows, expected,
                "configured={configured}, OOS={out_of_service}"
            );
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}
