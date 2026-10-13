//! LifeSafetyOperation service per ASHRAE 135-2020 Clause 13.13.

#[cfg(not(feature = "std"))]
use alloc::string::String;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_object_id, decode_ctx_primitive, decode_ctx_unsigned, decode_optional_ctx,
    expect_end,
};
use bacnet_encoding::primitives;
use bacnet_types::enums::LifeSafetyOperation;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

fn decode_requesting_source(content: &[u8]) -> Result<String, Error> {
    match content.first().copied() {
        Some(
            primitives::charset::IBM_MICROSOFT_DBCS
            | primitives::charset::JIS_X_0208
            | primitives::charset::UCS4,
        ) => Ok(String::new()),
        _ => primitives::decode_character_string(content),
    }
}

// ---------------------------------------------------------------------------
// LifeSafetyOperationRequest
// ---------------------------------------------------------------------------

/// LifeSafetyOperation-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeSafetyOperationRequest {
    /// Process on the requesting device that initiated the operation; meaning is local to that
    /// device.
    pub requesting_process_identifier: u32,
    /// Identity of the human operator behind the request. Decoding yields an empty string when the
    /// source uses a character set the stack cannot decode.
    pub requesting_source: String,
    /// Operation requested (silence, reset, unsilence and their audible/visual variants).
    pub request: LifeSafetyOperation,
    /// Single object the operation targets; `None` applies it to every applicable object in the
    /// receiving device.
    pub object_identifier: Option<ObjectIdentifier>,
}

impl LifeSafetyOperationRequest {
    /// Encode the request parameters into `buf`; fails if `requesting_source` cannot be encoded as
    /// a character string.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] requestingProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.requesting_process_identifier as u64);
        // [1] requestingSource
        primitives::encode_ctx_character_string(buf, 1, &self.requesting_source)?;
        // [2] request (BACnetLifeSafetyOperation)
        primitives::encode_ctx_enumerated(buf, 2, self.request.to_raw());
        // [3] objectIdentifier (optional)
        if let Some(ref oid) = self.object_identifier {
            primitives::encode_ctx_object_id(buf, 3, oid);
        }
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] requestingProcessIdentifier
        let (requesting_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "LifeSafetyOp processIdentifier")?;
        offset = end;

        // [1] requestingSource
        let (content, end) =
            decode_ctx_primitive(data, offset, 1, "LifeSafetyOp requestingSource")?;
        let requesting_source = decode_requesting_source(content)?;
        offset = end;

        // [2] request (BACnetLifeSafetyOperation)
        let (request_raw, end) =
            decode_ctx_unsigned::<u32>(data, offset, 2, "LifeSafetyOp request")?;
        let request = LifeSafetyOperation::from_raw(request_raw);
        offset = end;

        // [3] objectIdentifier (optional)
        let (object_identifier, end) = decode_optional_ctx(
            data,
            offset,
            3,
            "LifeSafetyOp objectIdentifier",
            decode_ctx_object_id,
        )?;
        expect_end(data, end, end, "LifeSafetyOp")?;

        Ok(Self {
            requesting_process_identifier,
            requesting_source,
            request,
            object_identifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::tags::{self as wire_tags, TagClass};
    use bacnet_types::enums::ObjectType;

    fn raw_context_unsigned(buf: &mut BytesMut, tag_number: u8, value: &[u8]) {
        wire_tags::encode_tag(buf, tag_number, TagClass::Context, value.len() as u32);
        buf.extend_from_slice(value);
    }

    fn raw_context_character_string(buf: &mut BytesMut, charset: u8, value: &[u8]) {
        wire_tags::encode_tag(buf, 1, TagClass::Context, (value.len() + 1) as u32);
        buf.extend_from_slice(&[charset]);
        buf.extend_from_slice(value);
    }

    fn raw_request_with_source(charset: u8, source: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        raw_context_unsigned(&mut buf, 0, &[1]);
        raw_context_character_string(&mut buf, charset, source);
        raw_context_unsigned(&mut buf, 2, &[LifeSafetyOperation::SILENCE.to_raw() as u8]);
        buf
    }

    fn raw_request(process_id: &[u8], operation: &[u8], field_tags: [u8; 3]) -> BytesMut {
        let mut buf = BytesMut::new();
        raw_context_unsigned(&mut buf, field_tags[0], process_id);
        primitives::encode_ctx_character_string(&mut buf, field_tags[1], "Panel-1").unwrap();
        raw_context_unsigned(&mut buf, field_tags[2], operation);
        buf
    }

    #[test]
    fn request_round_trip() {
        let req = LifeSafetyOperationRequest {
            requesting_process_identifier: 1,
            requesting_source: "Panel-1".into(),
            request: LifeSafetyOperation::SILENCE,
            object_identifier: Some(
                ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap(),
            ),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = LifeSafetyOperationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_no_oid_round_trip() {
        let req = LifeSafetyOperationRequest {
            requesting_process_identifier: 99,
            requesting_source: "Operator".into(),
            request: LifeSafetyOperation::RESET,
            object_identifier: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = LifeSafetyOperationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn unsupported_requesting_source_charsets_use_empty_string() {
        for charset in [1, 2, 3] {
            let decoded = LifeSafetyOperationRequest::decode(&raw_request_with_source(
                charset,
                &[0, 0, 0, 1],
            ))
            .unwrap();
            assert_eq!(decoded.requesting_source, "");
            assert_eq!(decoded.request, LifeSafetyOperation::SILENCE);
        }
    }

    #[test]
    fn malformed_supported_requesting_source_still_errors() {
        assert!(LifeSafetyOperationRequest::decode(&raw_request_with_source(0, &[0xff])).is_err());
    }

    #[test]
    fn request_values_must_fit_u32() {
        let max_with_leading_zero = [0, 0xff, 0xff, 0xff, 0xff];
        let decoded = LifeSafetyOperationRequest::decode(&raw_request(
            &max_with_leading_zero,
            &max_with_leading_zero,
            [0, 1, 2],
        ))
        .unwrap();
        assert_eq!(decoded.requesting_process_identifier, u32::MAX);
        assert_eq!(decoded.request.to_raw(), u32::MAX);

        let too_wide = [1, 0, 0, 0, 0];
        assert!(
            LifeSafetyOperationRequest::decode(&raw_request(&too_wide, &[0], [0, 1, 2],)).is_err()
        );
        assert!(
            LifeSafetyOperationRequest::decode(&raw_request(&[0], &too_wide, [0, 1, 2],)).is_err()
        );
    }

    #[test]
    fn request_requires_owned_context_tags() {
        for field in 0..3 {
            let mut field_tags = [0, 1, 2];
            field_tags[field] = 6;
            assert!(
                LifeSafetyOperationRequest::decode(&raw_request(&[1], &[1], field_tags)).is_err()
            );
        }

        let mut application_tagged = raw_request(&[1], &[1], [0, 1, 2]);
        application_tagged[0] &= !0x08;
        assert!(LifeSafetyOperationRequest::decode(&application_tagged).is_err());
    }

    #[test]
    fn request_rejects_unknown_optional_and_trailing_fields() {
        let mut unknown_optional = raw_request(&[1], &[1], [0, 1, 2]);
        primitives::encode_ctx_unsigned(&mut unknown_optional, 4, 1);
        assert!(LifeSafetyOperationRequest::decode(&unknown_optional).is_err());

        let mut trailing = raw_request(&[1], &[1], [0, 1, 2]);
        let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap();
        primitives::encode_ctx_object_id(&mut trailing, 3, &oid);
        primitives::encode_ctx_unsigned(&mut trailing, 4, 1);
        assert!(LifeSafetyOperationRequest::decode(&trailing).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_empty_input() {
        assert!(LifeSafetyOperationRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_truncated_1_byte() {
        let req = LifeSafetyOperationRequest {
            requesting_process_identifier: 1,
            requesting_source: "Panel-1".into(),
            request: LifeSafetyOperation::SILENCE,
            object_identifier: Some(
                ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap(),
            ),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(LifeSafetyOperationRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_truncated_half() {
        let req = LifeSafetyOperationRequest {
            requesting_process_identifier: 1,
            requesting_source: "Test".into(),
            request: LifeSafetyOperation::RESET_ALARM,
            object_identifier: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(LifeSafetyOperationRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_invalid_tag() {
        assert!(LifeSafetyOperationRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
