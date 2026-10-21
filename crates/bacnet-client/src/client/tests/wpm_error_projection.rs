use super::super::confirmed_response_result;
use crate::tsm::TsmResponse;
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, ErrorDetail};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;

#[test]
fn wpm_error_projection_exposes_the_first_failed_write_attempt() {
    // WritePropertyMultiple-Error as a peer sends it: [0] { PROPERTY (2),
    // WRITE_ACCESS_DENIED (40) }, then [1] around the reference to
    // (ANALOG_VALUE, 1) PRESENT_VALUE (85) index 3.
    let wire = [
        0x50, 7, 16, 0x0E, 0x91, 2, 0x91, 40, 0x0F, 0x1E, 0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 85,
        0x29, 3, 0x1F,
    ];
    let Apdu::Error(pdu) = decode_apdu(Bytes::copy_from_slice(&wire)).unwrap() else {
        panic!("expected an Error PDU");
    };
    let result = confirmed_response_result(TsmResponse::from_error_pdu(&pdu));
    let expected = BACnetObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: Some(3),
    };
    assert!(
        matches!(
            &result,
            Err(Error::Structured { class: ErrorClass::PROPERTY, code: ErrorCode::WRITE_ACCESS_DENIED, detail })
                if **detail == ErrorDetail::FirstFailedWriteAttempt(expected.clone())
        ),
        "{result:?}"
    );
}

#[test]
fn plain_error_projection_stays_class_and_code() {
    let result = confirmed_response_result(TsmResponse::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::WRITE_ACCESS_DENIED,
        detail: None,
    });
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
}
