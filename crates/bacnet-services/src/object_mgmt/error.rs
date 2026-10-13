//! CreateObject-Error (Clause 21), the error body of CreateObject. It has the
//! shape of ChangeList-Error.

use crate::common::error_type::{
    decode_element_error, decode_error_pdu, encode_element_error, error_pdu,
};
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::BytesMut;

/// The Result(-) body of CreateObject (Clause 15.3.1.3): the error, and the
/// 1-based position of the initial value in the request's List of Initial
/// Values that could not be applied, or 0 when the request failed for a
/// reason other than one of its initial values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateObjectError {
    /// Error class from the inner BACnetError.
    pub error_class: ErrorClass,
    /// Error code from the inner BACnetError.
    pub error_code: ErrorCode,
    /// Position of the failed initial value, starting at 1, or 0.
    pub first_failed_element_number: u32,
}

impl CreateObjectError {
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
            "CreateObject-Error",
            "CreateObject-Error first-failed-element-number",
        )?;
        Ok(Self {
            error_class,
            error_code,
            first_failed_element_number,
        })
    }

    /// Build the CreateObject Error PDU carrying this body.
    pub fn to_error_pdu(&self, invoke_id: u8) -> ErrorPdu {
        error_pdu(
            invoke_id,
            ConfirmedServiceChoice::CREATE_OBJECT,
            (self.error_class, self.error_code),
            |body| self.encode(body),
        )
    }
}

impl TryFrom<&ErrorPdu> for CreateObjectError {
    type Error = Error;

    /// Decode and verify the CreateObject-Error of a CreateObject Error PDU.
    /// A plain class/code error, which older devices send, is refused.
    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        decode_error_pdu(
            pdu,
            &[ConfirmedServiceChoice::CREATE_OBJECT],
            "CreateObject-Error",
            Self::decode,
            |error| (error.error_class, error.error_code),
        )
    }
}

impl From<CreateObjectError> for Error {
    fn from(error: CreateObjectError) -> Self {
        Error::protocol(
            error.error_class.to_raw() as u32,
            error.error_code.to_raw() as u32,
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

    fn sample(first_failed_element_number: u32) -> CreateObjectError {
        CreateObjectError {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            first_failed_element_number,
        }
    }

    #[test]
    fn create_object_error_round_trips_with_exact_shape() {
        // [0] around Enumerated 2 and 40, then [1] holding the element number
        // in its shortest form.
        for (number, tail) in [
            (0, &[0x19, 0][..]),
            (3, &[0x19, 3]),
            (300, &[0x1A, 1, 44]),
            (u32::MAX, &[0x1C, 0xFF, 0xFF, 0xFF, 0xFF]),
        ] {
            let error = sample(number);
            let mut body = BytesMut::new();
            error.encode(&mut body);
            assert_eq!(&body[..6], &[0x0E, 0x91, 2, 0x91, 40, 0x0F]);
            assert_eq!(&body[6..], tail);
            assert_eq!(CreateObjectError::decode(&body).unwrap(), error);
        }
    }

    #[test]
    fn create_object_error_pdu_conversion_and_projection() {
        let pdu = sample(2).to_error_pdu(9);
        assert_eq!(pdu.invoke_id, 9);
        assert_eq!(pdu.service_choice, ConfirmedServiceChoice::CREATE_OBJECT);
        assert_eq!(CreateObjectError::try_from(&pdu).unwrap(), sample(2));
        assert!(matches!(
            Error::from(sample(2)),
            Error::Structured { class: 2, code: 40, detail }
                if *detail == ErrorDetail::FirstFailedElementNumber(2)
        ));
    }

    #[test]
    fn create_object_error_refuses_malformed_plain_and_mismatched() {
        let mut body = BytesMut::new();
        sample(2).encode(&mut body);
        for malformed in [
            body[..6].to_vec(),
            [&body[..6], &[0x29, 2][..]].concat(),
            [&body[..], &[0x19, 2][..]].concat(),
            [&body[..6], &[0x1D, 5, 1, 0, 0, 0, 0][..]].concat(),
        ] {
            assert!(
                CreateObjectError::decode(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }

        let mut pdu = sample(1).to_error_pdu(1);
        pdu.error_code = ErrorCode::INVALID_DATA_TYPE;
        assert!(CreateObjectError::try_from(&pdu).is_err());
        let mut pdu = sample(1).to_error_pdu(1);
        pdu.error_data = Bytes::new();
        assert!(CreateObjectError::try_from(&pdu).is_err());
        let mut pdu = sample(1).to_error_pdu(1);
        pdu.service_choice = ConfirmedServiceChoice::ADD_LIST_ELEMENT;
        assert!(CreateObjectError::try_from(&pdu).is_err());
    }
}
