//! Formal WritePropertyMultiple Error service body (Clause 21).

use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::{
    decode_object_property_reference, encode_object_property_reference,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

/// Formal service-16 Result(-) body with the first failed write coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyMultipleError {
    /// Error class from the inner BACnetError.
    pub error_class: ErrorClass,
    /// Error code from the inner BACnetError.
    pub error_code: ErrorCode,
    /// Complete coordinate of the first failed write attempt.
    pub first_failed_write_attempt: BACnetObjectPropertyReference,
}

impl WritePropertyMultipleError {
    /// Encode `[0] Error` followed by `[1] BACnetObjectPropertyReference`.
    pub fn encode(&self, buf: &mut BytesMut) {
        tags::encode_opening_tag(buf, 0);
        primitives::encode_app_enumerated(buf, self.error_class.to_raw() as u32);
        primitives::encode_app_enumerated(buf, self.error_code.to_raw() as u32);
        tags::encode_closing_tag(buf, 0);
        tags::encode_opening_tag(buf, 1);
        encode_object_property_reference(buf, &self.first_failed_write_attempt);
        tags::encode_closing_tag(buf, 1);
    }

    /// Decode one complete formal service body with no trailing content.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (error_body, offset) = decode_constructed(data, 0, 0, "WPM Error [0]")?;
        let (error_class, error_code) = decode_error(error_body)?;
        let (reference_body, end) = decode_constructed(data, offset, 1, "WPM Error [1]")?;
        if end != data.len() {
            return Err(Error::decoding(end, "WPM Error has trailing content"));
        }
        let first_failed_write_attempt = decode_object_property_reference(reference_body)?;
        Ok(Self {
            error_class,
            error_code,
            first_failed_write_attempt,
        })
    }

    /// Build an existing low-level ErrorPdu while retaining the formal body.
    pub fn to_error_pdu(&self, invoke_id: u8) -> ErrorPdu {
        let mut body = BytesMut::new();
        self.encode(&mut body);
        ErrorPdu {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            error_class: self.error_class,
            error_code: self.error_code,
            error_data: body.freeze(),
        }
    }

    /// Decode and verify a formal WPM ErrorPdu.
    pub fn from_error_pdu(pdu: &ErrorPdu) -> Result<Self, Error> {
        Self::try_from(pdu)
    }
}

impl TryFrom<&ErrorPdu> for WritePropertyMultipleError {
    type Error = Error;

    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        if pdu.service_choice != ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE {
            return Err(Error::decoding(
                0,
                "ErrorPdu is not a WritePropertyMultiple result",
            ));
        }
        let decoded = Self::decode(&pdu.error_data)?;
        if decoded.error_class != pdu.error_class || decoded.error_code != pdu.error_code {
            return Err(Error::decoding(
                0,
                "formal WPM Error body disagrees with ErrorPdu class/code",
            ));
        }
        Ok(decoded)
    }
}

fn decode_constructed<'a>(
    data: &'a [u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if !tag.is_opening_tag(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("{field}: expected opening tag {tag_number}"),
        ));
    }
    tags::extract_context_value(data, content_start, tag_number)
}

fn decode_error(data: &[u8]) -> Result<(ErrorClass, ErrorCode), Error> {
    let (class, offset) = decode_enumerated(data, 0, "WPM error-class")?;
    let (code, end) = decode_enumerated(data, offset, "WPM error-code")?;
    if end != data.len() {
        return Err(Error::decoding(end, "WPM Error [0] has extra fields"));
    }
    Ok((ErrorClass::from_raw(class), ErrorCode::from_raw(code)))
}

fn decode_enumerated(data: &[u8], offset: usize, field: &str) -> Result<(u16, usize), Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            format!("{field}: expected application Enumerated"),
        ));
    }
    let end = content_start
        .checked_add(tag.length as usize)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| Error::decoding(content_start, format!("{field}: truncated payload")))?;
    let value = primitives::decode_unsigned(&data[content_start..end])?;
    let value = u16::try_from(value)
        .map_err(|_| Error::decoding(content_start, format!("{field}: value exceeds u16")))?;
    Ok((value, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    fn sample(index: Option<u32>) -> WritePropertyMultipleError {
        WritePropertyMultipleError {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            first_failed_write_attempt: BACnetObjectPropertyReference {
                object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 7).unwrap(),
                property_identifier: PropertyIdentifier::ACKED_TRANSITIONS.to_raw(),
                property_array_index: index,
            },
        }
    }

    #[test]
    fn indexed_and_unindexed_round_trip_with_exact_outer_shape() {
        for index in [None, Some(3)] {
            let error = sample(index);
            let mut body = BytesMut::new();
            error.encode(&mut body);
            assert_eq!(
                &body[..7],
                &[
                    0x0e,
                    0x91,
                    ErrorClass::PROPERTY.to_raw() as u8,
                    0x91,
                    ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u8,
                    0x0f,
                    0x1e,
                ]
            );
            assert_eq!(body.last(), Some(&0x1f));
            assert!(!body.iter().any(|byte| matches!(byte, 0x4e | 0x4f)));
            assert_eq!(WritePropertyMultipleError::decode(&body).unwrap(), error);
        }
    }

    #[test]
    fn error_pdu_conversion_retains_complete_formal_body() {
        let error = sample(None);
        let pdu = error.to_error_pdu(9);
        assert_eq!(pdu.invoke_id, 9);
        assert_eq!(pdu.error_data[0], 0x0e);
        assert_eq!(WritePropertyMultipleError::try_from(&pdu).unwrap(), error);
    }

    #[test]
    fn strict_decode_rejects_missing_duplicate_wrong_and_trailing_fields() {
        let error = sample(None);
        let mut body = BytesMut::new();
        error.encode(&mut body);

        for malformed in [
            body[..body.len() - 1].to_vec(),
            body[5..].to_vec(),
            {
                let mut bytes = body.to_vec();
                bytes[6] = 0x2e;
                bytes
            },
            {
                let mut bytes = body.to_vec();
                bytes.extend_from_slice(&[0x1e, 0x1f]);
                bytes
            },
            {
                let mut bytes = body.to_vec();
                bytes.splice(3..3, [0x91, 0x02]);
                bytes
            },
            {
                let mut bytes = body.to_vec();
                let property = bytes
                    .windows(2)
                    .rposition(|window| window == [0x19, 0x00])
                    .unwrap();
                bytes.drain(property..property + 2);
                bytes
            },
            {
                let mut bytes = body.to_vec();
                let property = bytes
                    .windows(2)
                    .rposition(|window| window == [0x19, 0x00])
                    .unwrap();
                bytes.splice(property..property, [0x19, 0x00]);
                bytes
            },
            {
                let mut bytes = body.to_vec();
                let property = bytes
                    .windows(2)
                    .rposition(|window| window == [0x19, 0x00])
                    .unwrap();
                bytes[property] = 0x29;
                bytes
            },
        ] {
            assert!(WritePropertyMultipleError::decode(&malformed).is_err());
        }
    }

    #[test]
    fn conversion_rejects_mismatched_projection() {
        let error = sample(None);
        let mut pdu = error.to_error_pdu(1);
        pdu.error_code = ErrorCode::UNKNOWN_PROPERTY;
        assert!(WritePropertyMultipleError::from_error_pdu(&pdu).is_err());

        pdu.error_data = Bytes::new();
        assert!(WritePropertyMultipleError::from_error_pdu(&pdu).is_err());
    }
}
