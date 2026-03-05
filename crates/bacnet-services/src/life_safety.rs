//! LifeSafetyOperation service per ASHRAE 135-2020 Clause 15.2.7.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::LifeSafetyOperation;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// LifeSafetyOperationRequest (Clause 15.2.7)
// ---------------------------------------------------------------------------

/// LifeSafetyOperation-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeSafetyOperationRequest {
    pub requesting_process_identifier: u32,
    pub requesting_source: String,
    pub request: LifeSafetyOperation,
    pub object_identifier: Option<ObjectIdentifier>,
}

impl LifeSafetyOperationRequest {
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

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] requestingProcessIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "LifeSafetyOp truncated at processIdentifier",
            ));
        }
        let requesting_process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] requestingSource
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "LifeSafetyOp truncated at requestingSource",
            ));
        }
        let requesting_source = primitives::decode_character_string(&data[pos..end])?;
        offset = end;

        // [2] request (BACnetLifeSafetyOperation)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "LifeSafetyOp truncated at request"));
        }
        let request =
            LifeSafetyOperation::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [3] objectIdentifier (optional)
        let mut object_identifier = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt_data {
                object_identifier = Some(ObjectIdentifier::decode(content)?);
            }
        }

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
    use bacnet_types::enums::ObjectType;

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
