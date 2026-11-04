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
    .encode(&mut buf)
    .unwrap();
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
        PropertyValue::List(Vec::new()),
        // A constructed value, already encoded: `[0]` opening, an Unsigned,
        // `[0]` closing.
        PropertyValue::ApplicationData(vec![0x0E, 0x21, 0x05, 0x0F]),
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

/// An Event_Parameters value in the legacy `[255]` form (#2376).
fn event_parameters(oid: ObjectIdentifier) -> ReadPropertyRequest {
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
        property_array_index: None,
    }
}

#[test]
fn a_legacy_value_that_decodes_back_is_encoded_as_the_service_encoder_does() {
    let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
    let request = event_parameters(oid);
    let value = PropertyValue::ApplicationData(vec![0xFE, 0xFF, 0x21, 0x05, 0xFF, 0xFF]);
    let mut buf = BytesMut::new();
    encode_ack(&mut buf, oid, &request, &value).unwrap();
    assert_eq!(&buf[..], reference(&value, oid, &request).as_slice());
}

#[test]
fn a_refused_value_leaves_the_buffer_as_it_was() {
    let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
    let request = event_parameters(oid);
    // A legacy value with no closing pair wouldn't decode back.
    let value = PropertyValue::ApplicationData(vec![0xFE, 0xFF, 1, 2]);
    let before = [0x30, 9, 0x0C];
    let mut buf = BytesMut::from(&before[..]);
    let error = encode_ack(&mut buf, oid, &request, &value).unwrap_err();
    assert_eq!(&buf[..], before);
    // The same refusal, and so the same Error PDU, as the service encoder's.
    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, &value).unwrap();
    let service = ReadPropertyACK {
        object_identifier: oid,
        property_identifier: request.property_identifier,
        property_array_index: None,
        property_value: value_buf.to_vec(),
    }
    .encode(&mut BytesMut::new())
    .unwrap_err();
    assert_eq!(error.to_string(), service.to_string());
}
