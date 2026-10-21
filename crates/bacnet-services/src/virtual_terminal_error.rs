//! VTClose-Error (Clause 21): `[0]` Error, then an optional `[1]` frame of
//! application Unsigned8 VT session identifiers.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::common::error_type::{
    decode_error_pdu, decode_error_type, encode_error_type, error_pdu,
};
use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::tagged::{
    decode_app_unsigned, decode_ctx_constructed, expect_end,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::BytesMut;

const WHAT: &str = "VTClose-Error";

/// The Result(-) body of VT-Close (Clause 17.3.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTCloseError {
    /// Error class from the inner BACnetError.
    pub error_class: ErrorClass,
    /// Error code from the inner BACnetError.
    pub error_code: ErrorCode,
    /// The requester's local identifiers of the VT sessions that could not
    /// be closed. The responder includes the list with
    /// VT_SESSION_TERMINATION_FAILURE and omits it (`None`) otherwise.
    pub list_of_vt_session_identifiers: Option<Vec<u8>>,
}

impl VTCloseError {
    /// Encode the error, then the session list when present.
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_error_type(buf, self.error_class, self.error_code);
        if let Some(sessions) = &self.list_of_vt_session_identifiers {
            tags::encode_opening_tag(buf, 1);
            for &session in sessions {
                primitives::encode_app_unsigned(buf, u64::from(session));
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    /// Decode one complete body with no trailing content.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let ((error_class, error_code), mut offset) = decode_error_type(data, WHAT)?;
        let mut list_of_vt_session_identifiers = None;
        if offset < data.len() {
            let (frame, end) = decode_ctx_constructed(data, offset, 1, WHAT)?;
            let mut sessions = Vec::new();
            let mut position = 0;
            while position < frame.len() {
                if sessions.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::overflow(position, "VTClose-Error too many sessions"));
                }
                let (session, next) =
                    decode_app_unsigned::<u8>(frame, position, "VTClose-Error session-identifier")?;
                sessions.push(session);
                position = next;
            }
            list_of_vt_session_identifiers = Some(sessions);
            offset = end;
        }
        expect_end(data, offset, offset, WHAT)?;
        Ok(Self {
            error_class,
            error_code,
            list_of_vt_session_identifiers,
        })
    }

    /// Build the VT-Close Error PDU carrying this body.
    pub fn to_error_pdu(&self, invoke_id: u8) -> ErrorPdu {
        error_pdu(
            invoke_id,
            ConfirmedServiceChoice::VT_CLOSE,
            (self.error_class, self.error_code),
            |body| self.encode(body),
        )
    }
}

impl TryFrom<&ErrorPdu> for VTCloseError {
    type Error = Error;

    /// Decode and verify the body of a VT-Close Error PDU. A plain class/code
    /// error, which older devices send, is refused.
    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        decode_error_pdu(
            pdu,
            &[ConfirmedServiceChoice::VT_CLOSE],
            WHAT,
            Self::decode,
            |error| (error.error_class, error.error_code),
        )
    }
}

impl From<VTCloseError> for Error {
    fn from(error: VTCloseError) -> Self {
        Error::protocol(
            error.error_class,
            error.error_code,
            error
                .list_of_vt_session_identifiers
                .map(ErrorDetail::VtSessionIdentifiers),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn sample(sessions: Option<Vec<u8>>) -> VTCloseError {
        VTCloseError {
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::VT_SESSION_TERMINATION_FAILURE,
            list_of_vt_session_identifiers: sessions,
        }
    }

    fn encoded(error: &VTCloseError) -> Vec<u8> {
        let mut body = BytesMut::new();
        error.encode(&mut body);
        body.to_vec()
    }

    #[test]
    fn vt_close_error_round_trips_with_exact_shape() {
        // [0] { 5, 39 }, then [1] around application Unsigned (0x21) values.
        let head = [0x0E, 0x91, 5, 0x91, 39, 0x0F];
        for (sessions, tail) in [
            (None, &[][..]),
            (Some(vec![1, 4]), &[0x1E, 0x21, 1, 0x21, 4, 0x1F]),
            (Some(vec![255]), &[0x1E, 0x21, 255, 0x1F]),
        ] {
            let error = sample(sessions);
            let wire = [&head[..], tail].concat();
            assert_eq!(encoded(&error), wire);
            assert_eq!(VTCloseError::decode(&wire).unwrap(), error);
            let pdu = error.to_error_pdu(2);
            assert_eq!(VTCloseError::try_from(&pdu).unwrap(), error);
        }
    }

    #[test]
    fn only_a_present_list_is_a_detail() {
        assert!(matches!(
            Error::from(sample(None)),
            Error::Protocol { class: ErrorClass::SERVICES, code: ErrorCode::VT_SESSION_TERMINATION_FAILURE }
        ));
        assert!(matches!(
            Error::from(sample(Some(vec![1, 4]))),
            Error::Structured { class: ErrorClass::SERVICES, code: ErrorCode::VT_SESSION_TERMINATION_FAILURE, detail }
                if *detail == ErrorDetail::VtSessionIdentifiers(vec![1, 4])
        ));
    }

    #[test]
    fn vt_close_error_refuses_malformed_plain_and_mismatched() {
        let body = encoded(&sample(Some(vec![1, 4])));
        for malformed in [
            body[..body.len() - 1].to_vec(),
            [&body[..], &[0x00][..]].concat(),
            [&body[..6], &[0x21, 1][..]].concat(),
            [&body[..6], &[0x1E, 0x22, 1, 0, 0x1F][..]].concat(),
            [&body[..6], &[0x1E, 0x09, 1, 0x1F][..]].concat(),
        ] {
            assert!(
                VTCloseError::decode(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }

        let mut pdu = sample(None).to_error_pdu(1);
        pdu.error_code = ErrorCode::UNKNOWN_VT_SESSION;
        assert!(VTCloseError::try_from(&pdu).is_err());
        let mut pdu = sample(None).to_error_pdu(1);
        pdu.error_data = Bytes::new();
        assert!(VTCloseError::try_from(&pdu).is_err());
        let mut pdu = sample(None).to_error_pdu(1);
        pdu.service_choice = ConfirmedServiceChoice::VT_OPEN;
        assert!(VTCloseError::try_from(&pdu).is_err());
    }
}
