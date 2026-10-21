//! ConfirmedPrivateTransfer-Error (Clause 21): `[0]` Error, `[1]` vendor
//! identifier, `[2]` service number and optional `[3]` error parameters.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::common::error_type::{
    decode_error_pdu, decode_error_type, encode_error_type, error_pdu,
};
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_constructed, decode_ctx_unsigned, expect_end,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::{BufMut, BytesMut};

const WHAT: &str = "ConfirmedPrivateTransfer-Error";

/// The Result(-) body of ConfirmedPrivateTransfer (Clause 16.2.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateTransferError {
    /// Error class from the inner BACnetError.
    pub error_class: ErrorClass,
    /// Error code from the inner BACnetError.
    pub error_code: ErrorCode,
    /// Vendor identifier of the private service the error answers.
    pub vendor_id: u32,
    /// Vendor-defined service number the error answers.
    pub service_number: u32,
    /// Vendor-defined error parameters (raw bytes inside the `[3]` frame,
    /// opaque to the stack); `None` when absent.
    pub error_parameters: Option<Vec<u8>>,
}

impl PrivateTransferError {
    /// Encode the body: the error, vendor, service and any parameters.
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_error_type(buf, self.error_class, self.error_code);
        primitives::encode_ctx_unsigned(buf, 1, u64::from(self.vendor_id));
        primitives::encode_ctx_unsigned(buf, 2, u64::from(self.service_number));
        if let Some(parameters) = &self.error_parameters {
            tags::encode_opening_tag(buf, 3);
            buf.put_slice(parameters);
            tags::encode_closing_tag(buf, 3);
        }
    }

    /// Decode one complete body with no trailing content.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let ((error_class, error_code), offset) = decode_error_type(data, WHAT)?;
        let (vendor_id, offset) = decode_ctx_unsigned::<u32>(
            data,
            offset,
            1,
            "ConfirmedPrivateTransfer-Error vendor-id",
        )?;
        let (service_number, mut offset) = decode_ctx_unsigned::<u32>(
            data,
            offset,
            2,
            "ConfirmedPrivateTransfer-Error service-number",
        )?;
        let mut error_parameters = None;
        if offset < data.len() {
            let (parameters, end) = decode_ctx_constructed(data, offset, 3, WHAT)?;
            error_parameters = Some(parameters.to_vec());
            offset = end;
        }
        expect_end(data, offset, offset, WHAT)?;
        Ok(Self {
            error_class,
            error_code,
            vendor_id,
            service_number,
            error_parameters,
        })
    }

    /// Build the ConfirmedPrivateTransfer Error PDU carrying this body.
    pub fn to_error_pdu(&self, invoke_id: u8) -> ErrorPdu {
        error_pdu(
            invoke_id,
            ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER,
            (self.error_class, self.error_code),
            |body| self.encode(body),
        )
    }
}

impl TryFrom<&ErrorPdu> for PrivateTransferError {
    type Error = Error;

    /// Decode and verify the body of a ConfirmedPrivateTransfer Error PDU. A
    /// plain class/code error, which older devices send, is refused.
    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        decode_error_pdu(
            pdu,
            &[ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER],
            WHAT,
            Self::decode,
            |error| (error.error_class, error.error_code),
        )
    }
}

impl From<PrivateTransferError> for Error {
    fn from(error: PrivateTransferError) -> Self {
        Error::protocol(
            error.error_class,
            error.error_code,
            Some(ErrorDetail::PrivateTransfer {
                vendor_id: error.vendor_id,
                service_number: error.service_number,
                error_parameters: error.error_parameters,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn sample(error_parameters: Option<Vec<u8>>) -> PrivateTransferError {
        PrivateTransferError {
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            vendor_id: 555,
            service_number: 7,
            error_parameters,
        }
    }

    fn encoded(error: &PrivateTransferError) -> Vec<u8> {
        let mut body = BytesMut::new();
        error.encode(&mut body);
        body.to_vec()
    }

    #[test]
    fn private_transfer_error_round_trips_with_exact_shape() {
        // [0] { 5, 45 }, [1] vendor 555 in two octets, [2] service 7, then
        // [3] around the parameters when present (empty is kept as empty).
        let head = [0x0E, 0x91, 5, 0x91, 45, 0x0F, 0x1A, 0x02, 0x2B, 0x29, 7];
        for (parameters, tail) in [
            (None, &[][..]),
            (Some(vec![0x21, 1]), &[0x3E, 0x21, 1, 0x3F]),
            (Some(Vec::new()), &[0x3E, 0x3F]),
            (
                Some(vec![0x3E, 0x21, 1, 0x3F]),
                &[0x3E, 0x3E, 0x21, 1, 0x3F, 0x3F],
            ),
        ] {
            let error = sample(parameters);
            let wire = [&head[..], tail].concat();
            assert_eq!(encoded(&error), wire);
            assert_eq!(PrivateTransferError::decode(&wire).unwrap(), error);
            let pdu = error.to_error_pdu(3);
            assert_eq!(PrivateTransferError::try_from(&pdu).unwrap(), error);
        }
    }

    #[test]
    fn private_transfer_error_projects_vendor_service_and_parameters() {
        assert!(matches!(
            Error::from(sample(Some(vec![0x21, 1]))),
            Error::Structured { class: ErrorClass::SERVICES, code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, detail }
                if *detail == ErrorDetail::PrivateTransfer {
                    vendor_id: 555,
                    service_number: 7,
                    error_parameters: Some(vec![0x21, 1]),
                }
        ));
    }

    #[test]
    fn private_transfer_error_refuses_malformed_plain_and_mismatched() {
        let body = encoded(&sample(Some(vec![0x21, 1])));
        for malformed in [
            body[..6].to_vec(),
            body[..9].to_vec(),
            body[..body.len() - 1].to_vec(),
            [&body[..], &[0x00][..]].concat(),
            [&body[..6], &[0x29, 7, 0x1A, 0x02, 0x2B][..]].concat(),
            [&body[..11], &[0x39, 1][..]].concat(),
        ] {
            assert!(
                PrivateTransferError::decode(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }

        let mut pdu = sample(None).to_error_pdu(1);
        pdu.error_class = ErrorClass::DEVICE;
        assert!(PrivateTransferError::try_from(&pdu).is_err());
        let mut pdu = sample(None).to_error_pdu(1);
        pdu.error_data = Bytes::new();
        assert!(PrivateTransferError::try_from(&pdu).is_err());
        let mut pdu = sample(None).to_error_pdu(1);
        pdu.service_choice = ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE;
        assert!(PrivateTransferError::try_from(&pdu).is_err());
    }
}
