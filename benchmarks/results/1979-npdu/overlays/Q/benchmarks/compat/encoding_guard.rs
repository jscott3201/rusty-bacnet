// Independent literals derived in rusty-1979-independent-oracles.md, outside timing.
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

#[test]
fn independent_npdu_bytes_and_decoded_fields_match_original_workload() {
    let npdu = Npdu {
        payload: Bytes::from(vec![0; 100]),
        ..Default::default()
    };
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, &npdu).unwrap();
    let mut expected = vec![0; 102];
    expected[0] = 1;
    assert_eq!(encoded.as_ref(), expected);
    let decoded = decode_npdu(Bytes::from(expected)).unwrap();
    assert!(!decoded.expecting_reply);
    assert_eq!(decoded.priority.to_raw(), 0);
    assert!(decoded.destination.is_none());
    assert!(decoded.source.is_none());
    assert_eq!(decoded.hop_count, 255);
    assert!(decoded.message_type.is_none());
    assert!(decoded.vendor_id.is_none());
    assert_eq!(decoded.payload.as_ref(), &[0; 100]);
}

#[test]
fn independent_full_request_bytes_match_original_allocating_workload() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut service = BytesMut::with_capacity(32);
    request.encode(&mut service);
    assert_eq!(service.as_ref(), &[0x0c, 0, 0, 0, 1, 0x19, 0x55]);
    let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from(service.to_vec()),
    });
    let mut apdu_bytes = BytesMut::with_capacity(64);
    encode_apdu(&mut apdu_bytes, &apdu).unwrap();
    let npdu = Npdu {
        payload: Bytes::from(apdu_bytes.to_vec()),
        ..Default::default()
    };
    let mut encoded = BytesMut::with_capacity(128);
    encode_npdu(&mut encoded, &npdu).unwrap();
    let expected = [
        0x01, 0x00, 0x02, 0x05, 0x01, 0x0c, 0x0c, 0, 0, 0, 1, 0x19, 0x55,
    ];
    assert_eq!(encoded.as_ref(), &expected);
    let decoded = decode_npdu(Bytes::copy_from_slice(&expected)).unwrap();
    let Apdu::ConfirmedRequest(decoded) = decode_apdu(decoded.payload).unwrap() else {
        panic!("expected confirmed request")
    };
    assert_eq!(decoded.invoke_id, 1);
    assert_eq!(
        decoded.service_choice,
        ConfirmedServiceChoice::READ_PROPERTY
    );
    assert_eq!(decoded.max_apdu_length, 1476);
    let decoded = ReadPropertyRequest::decode(&decoded.service_request).unwrap();
    assert_eq!(decoded.object_identifier, oid);
    assert_eq!(
        decoded.property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert!(decoded.property_array_index.is_none());
}

#[test]
fn independent_ack_service_and_real_value_are_preserved() {
    let expected = [
        0x0c, 0, 0, 0, 1, 0x19, 0x55, 0x3e, 0x44, 0x41, 0xa0, 0xcc, 0xcd, 0x3f,
    ];
    let ack = ReadPropertyACK::decode(&expected).unwrap();
    assert_eq!(
        ack.object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
    );
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert!(ack.property_array_index.is_none());
    assert_eq!(ack.property_value, [0x44, 0x41, 0xa0, 0xcc, 0xcd]);
    assert_eq!((20.0_f32 + 0.1_f32).to_bits(), 0x41a0cccd);
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded);
    assert_eq!(encoded.as_ref(), expected);
}
