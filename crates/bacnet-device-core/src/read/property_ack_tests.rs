//! The ReadProperty-ACK encoded in place is the service encoder's output,
//! byte for byte (#1978 item 3).

use super::*;
use bacnet_services::read_property::ReadPropertyACK;

fn reference(
    value: &PropertyValue,
    oid: ObjectIdentifier,
    request: &ReadPropertyRequest,
) -> Vec<u8> {
    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, value).unwrap();
    let mut buf = BytesMut::new();
    ReadPropertyACK {
        object_identifier: oid,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: value_buf.to_vec(),
    }
    .encode(&mut buf);
    buf.to_vec()
}

#[test]
fn the_ack_encoded_in_place_matches_the_service_encoder() {
    let values = [
        PropertyValue::Real(21.0),
        PropertyValue::Null,
        PropertyValue::Boolean(true),
        PropertyValue::Unsigned(u64::from(u32::MAX)),
        PropertyValue::CharacterString("x".repeat(300)),
        PropertyValue::OctetString(vec![0xAB; 40]),
        PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Real(2.5)]),
    ];
    let oids = [
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE - 1).unwrap(),
        ObjectIdentifier::new(ObjectType::from_raw(1000), 4_000_000).unwrap(),
    ];
    let properties = [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::from_raw(4_000_000),
    ];
    for value in &values {
        for &oid in &oids {
            for &property_identifier in &properties {
                for property_array_index in [None, Some(0), Some(7), Some(u32::MAX)] {
                    let request = ReadPropertyRequest {
                        object_identifier: oid,
                        property_identifier,
                        property_array_index,
                    };
                    // Appended after what the buffer already holds.
                    let mut buf = BytesMut::from(&[0x30, 9, 0x0C][..]);
                    encode_ack(&mut buf, oid, &request, value).unwrap();
                    assert_eq!(&buf[..3], [0x30, 9, 0x0C]);
                    assert_eq!(
                        &buf[3..],
                        reference(value, oid, &request).as_slice(),
                        "{value:?} {oid:?} {property_identifier:?} {property_array_index:?}"
                    );
                }
            }
        }
    }
}
