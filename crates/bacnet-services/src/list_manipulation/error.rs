//! ChangeList-Error (Clause 21), the error body of AddListElement and
//! RemoveListElement.

use crate::common::error_type::{
    decode_element_error, decode_error_pdu, encode_element_error, error_pdu,
};
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::BytesMut;

const SERVICES: [ConfirmedServiceChoice; 2] = [
    ConfirmedServiceChoice::ADD_LIST_ELEMENT,
    ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
];

/// The Result(-) body of AddListElement and RemoveListElement (Clauses
/// 15.1.1.3 and 15.2.1.3): the error, and the 1-based position of the element
/// of the request's List of Elements that failed, or 0 when the request failed
/// for a reason other than one of its elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeListError {
    /// Error class from the inner BACnetError.
    pub error_class: ErrorClass,
    /// Error code from the inner BACnetError.
    pub error_code: ErrorCode,
    /// Position of the failed element, starting at 1, or 0.
    pub first_failed_element_number: u32,
}

impl ChangeListError {
    /// Encode `[0] Error` followed by `[1]` first-failed-element-number.
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_element_error(
            buf,
            self.error_class,
            self.error_code,
            self.first_failed_element_number,
        );
    }

    /// Decode one complete body with no trailing content.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (error_class, error_code, first_failed_element_number) = decode_element_error(
            data,
            "ChangeList-Error",
            "ChangeList-Error first-failed-element-number",
        )?;
        Ok(Self {
            error_class,
            error_code,
            first_failed_element_number,
        })
    }

    /// Build the Error PDU answering `service_choice`, which should be
    /// AddListElement or RemoveListElement: for any other service the APDU
    /// encoder sends the body as opaque data after a plain class and code.
    pub fn to_error_pdu(&self, invoke_id: u8, service_choice: ConfirmedServiceChoice) -> ErrorPdu {
        debug_assert!(SERVICES.contains(&service_choice), "{service_choice:?}");
        error_pdu(
            invoke_id,
            service_choice,
            (self.error_class, self.error_code),
            |body| self.encode(body),
        )
    }
}

impl TryFrom<&ErrorPdu> for ChangeListError {
    type Error = Error;

    /// Decode and verify the ChangeList-Error of an AddListElement or
    /// RemoveListElement Error PDU. A plain class/code error, which older
    /// devices send, has no element number and is refused.
    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        decode_error_pdu(pdu, &SERVICES, "ChangeList-Error", Self::decode, |error| {
            (error.error_class, error.error_code)
        })
    }
}

impl From<ChangeListError> for Error {
    fn from(error: ChangeListError) -> Self {
        Error::protocol(
            error.error_class,
            error.error_code,
            Some(ErrorDetail::FirstFailedElementNumber(
                error.first_failed_element_number,
            )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn sample(first_failed_element_number: u32) -> ChangeListError {
        ChangeListError {
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::LIST_ELEMENT_NOT_FOUND,
            first_failed_element_number,
        }
    }

    #[test]
    fn change_list_error_round_trips_with_exact_shape() {
        // [0] around Enumerated 5 and 81, then [1] holding the element number
        // in its shortest form (one octet up to 255, four at u32::MAX).
        for (number, tail) in [
            (0, &[0x19, 0][..]),
            (3, &[0x19, 3]),
            (300, &[0x1A, 1, 44]),
            (u32::MAX, &[0x1C, 0xFF, 0xFF, 0xFF, 0xFF]),
        ] {
            let error = sample(number);
            let mut body = BytesMut::new();
            error.encode(&mut body);
            assert_eq!(&body[..6], &[0x0E, 0x91, 5, 0x91, 81, 0x0F]);
            assert_eq!(&body[6..], tail);
            assert_eq!(ChangeListError::decode(&body).unwrap(), error);
        }
    }

    #[test]
    fn change_list_error_pdu_conversion_keeps_both_services() {
        for service in [
            ConfirmedServiceChoice::ADD_LIST_ELEMENT,
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
        ] {
            let pdu = sample(2).to_error_pdu(9, service);
            assert_eq!(pdu.invoke_id, 9);
            assert_eq!(pdu.service_choice, service);
            assert_eq!(ChangeListError::try_from(&pdu).unwrap(), sample(2));
            assert!(matches!(
                Error::from(sample(2)),
                Error::Structured { class: ErrorClass::SERVICES, code: ErrorCode::LIST_ELEMENT_NOT_FOUND, detail }
                    if *detail == ErrorDetail::FirstFailedElementNumber(2)
            ));
        }
    }

    #[test]
    fn change_list_error_decode_rejects_missing_wrong_and_trailing_fields() {
        let mut body = BytesMut::new();
        sample(2).encode(&mut body);
        for malformed in [
            body[..6].to_vec(),
            body[6..].to_vec(),
            [&body[..6], &[0x29, 2][..]].concat(),
            [&body[..6], &[0x1E, 0x21, 2, 0x1F][..]].concat(),
            [&body[..6], &[0x1D, 5, 1, 0, 0, 0, 0][..]].concat(),
            [&body[..], &[0x19, 2][..]].concat(),
            [&[0x0E, 0x91, 5][..], &body[5..]].concat(),
        ] {
            assert!(
                ChangeListError::decode(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }
    }

    #[test]
    fn change_list_error_conversion_rejects_plain_and_mismatched_pdus() {
        let mut pdu = sample(1).to_error_pdu(1, ConfirmedServiceChoice::ADD_LIST_ELEMENT);
        pdu.error_code = ErrorCode::INVALID_DATA_TYPE;
        assert!(ChangeListError::try_from(&pdu).is_err());

        let mut pdu = sample(1).to_error_pdu(1, ConfirmedServiceChoice::REMOVE_LIST_ELEMENT);
        pdu.error_data = Bytes::new();
        assert!(ChangeListError::try_from(&pdu).is_err());

        let mut pdu = sample(1).to_error_pdu(1, ConfirmedServiceChoice::ADD_LIST_ELEMENT);
        pdu.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
        assert!(ChangeListError::try_from(&pdu).is_err());
    }
}
