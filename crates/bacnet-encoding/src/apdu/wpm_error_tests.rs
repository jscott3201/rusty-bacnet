use super::*;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

fn encode_to_vec(apdu: &Apdu) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, apdu).unwrap();
    bytes.to_vec()
}

fn formal_body(class: ErrorClass, code: ErrorCode, indexed: bool) -> Bytes {
    let mut body = BytesMut::new();
    tags::encode_opening_tag(&mut body, 0);
    primitives::encode_app_enumerated(&mut body, class.to_raw() as u32);
    primitives::encode_app_enumerated(&mut body, code.to_raw() as u32);
    tags::encode_closing_tag(&mut body, 0);
    tags::encode_opening_tag(&mut body, 1);
    crate::constructed::encode_object_property_reference(
        &mut body,
        &BACnetObjectPropertyReference {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 4).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: indexed.then_some(8),
        },
    );
    tags::encode_closing_tag(&mut body, 1);
    body.freeze()
}

#[test]
fn formal_wpm_error_round_trip_does_not_prepend_generic_pair() {
    for indexed in [false, true] {
        let body = formal_body(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
            indexed,
        );
        let pdu = ErrorPdu {
            invoke_id: 11,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            error_data: body.clone(),
        };
        let encoded = encode_to_vec(&Apdu::Error(pdu.clone()));
        assert_eq!(&encoded[..3], &[0x50, 11, 16]);
        assert_eq!(&encoded[3..], body.as_ref());
        assert_eq!(decode_apdu(Bytes::from(encoded)).unwrap(), Apdu::Error(pdu));
    }
}

#[test]
fn legacy_generic_wpm_error_still_decodes_and_encodes() {
    let pdu = ErrorPdu {
        invoke_id: 12,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        error_data: Bytes::new(),
    };
    let encoded = encode_to_vec(&Apdu::Error(pdu.clone()));
    assert_eq!(encoded[3], 0x91);
    assert_eq!(decode_apdu(Bytes::from(encoded)).unwrap(), Apdu::Error(pdu));
}

#[test]
fn legacy_generic_wpm_context_zero_data_still_round_trips() {
    let original = Apdu::Error(ErrorPdu {
        invoke_id: 4,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        error_class: ErrorClass::SERVICES,
        error_code: ErrorCode::OTHER,
        error_data: Bytes::from_static(&[0x0e, 0x0f]),
    });
    let mut wire = BytesMut::new();
    encode_apdu(&mut wire, &original).unwrap();
    assert_eq!(decode_apdu(wire.freeze()).unwrap(), original);
}

#[test]
fn formal_wpm_error_projection_must_match_body() {
    let pdu = Apdu::Error(ErrorPdu {
        invoke_id: 1,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::UNKNOWN_PROPERTY,
        error_data: formal_body(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED, false),
    });
    let mut encoded = BytesMut::new();
    assert!(encode_apdu(&mut encoded, &pdu).is_err());
    assert!(encoded.is_empty());
}

#[test]
fn generic_non_wpm_error_behavior_is_unchanged() {
    let pdu = ErrorPdu {
        invoke_id: 13,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::UNKNOWN_PROPERTY,
        error_data: Bytes::from_static(&[0x0e, 0x0f]),
    };
    let encoded = encode_to_vec(&Apdu::Error(pdu.clone()));
    assert_eq!(encoded[3], 0x91);
    assert_eq!(decode_apdu(Bytes::from(encoded)).unwrap(), Apdu::Error(pdu));
}
