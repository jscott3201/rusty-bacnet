//! Who-Has and I-Have services per ASHRAE 135-2020 Clause 16.9.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// WhoHasRequest
// ---------------------------------------------------------------------------

/// The object to search for: by identifier or by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhoHasObject {
    /// Search by object identifier ([2] context tag).
    Identifier(ObjectIdentifier),
    /// Search by object name ([3] context tag).
    Name(String),
}

/// Who-Has-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoHasRequest {
    pub low_limit: Option<u32>,
    pub high_limit: Option<u32>,
    pub object: WhoHasObject,
}

impl WhoHasRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] low-limit (optional)
        if let Some(low) = self.low_limit {
            primitives::encode_ctx_unsigned(buf, 0, low as u64);
        }
        // [1] high-limit (optional)
        if let Some(high) = self.high_limit {
            primitives::encode_ctx_unsigned(buf, 1, high as u64);
        }
        // CHOICE: [2] object-identifier OR [3] object-name
        match &self.object {
            WhoHasObject::Identifier(oid) => {
                primitives::encode_ctx_object_id(buf, 2, oid);
            }
            WhoHasObject::Name(name) => {
                primitives::encode_ctx_character_string(buf, 3, name)?;
            }
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] low-limit (optional)
        let mut low_limit = None;
        let (opt, new_offset) = tags::decode_optional_context(data, offset, 0)?;
        if let Some(content) = opt {
            low_limit = Some(primitives::decode_unsigned(content)? as u32);
            offset = new_offset;
        }

        // [1] high-limit (optional)
        let mut high_limit = None;
        let (opt, new_offset) = tags::decode_optional_context(data, offset, 1)?;
        if let Some(content) = opt {
            high_limit = Some(primitives::decode_unsigned(content)? as u32);
            offset = new_offset;
        }

        // CHOICE: [2] object-identifier OR [3] object-name
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "WhoHas truncated at object choice"));
        }

        let object = if tag.is_context(2) {
            WhoHasObject::Identifier(ObjectIdentifier::decode(&data[pos..end])?)
        } else if tag.is_context(3) {
            let s = primitives::decode_character_string(&data[pos..end])?;
            WhoHasObject::Name(s)
        } else {
            return Err(Error::decoding(
                offset,
                "WhoHas expected context tag 2 or 3",
            ));
        };

        Ok(Self {
            low_limit,
            high_limit,
            object,
        })
    }
}

// ---------------------------------------------------------------------------
// IHaveRequest
// ---------------------------------------------------------------------------

/// I-Have-Request service parameters (APPLICATION-tagged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IHaveRequest {
    pub device_identifier: ObjectIdentifier,
    pub object_identifier: ObjectIdentifier,
    pub object_name: String,
}

impl IHaveRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        primitives::encode_app_object_id(buf, &self.device_identifier);
        primitives::encode_app_object_id(buf, &self.object_identifier);
        primitives::encode_app_character_string(buf, &self.object_name)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IHave truncated at device-id"));
        }
        let device_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IHave truncated at object-id"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IHave truncated at object-name"));
        }
        let object_name = primitives::decode_character_string(&data[pos..end])?;

        Ok(Self {
            device_identifier,
            object_identifier,
            object_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn who_has_by_id_round_trip() {
        let req = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Identifier(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            ),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = WhoHasRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn who_has_by_name_with_limits() {
        let req = WhoHasRequest {
            low_limit: Some(1000),
            high_limit: Some(2000),
            object: WhoHasObject::Name("Zone Temperature".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = WhoHasRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn i_have_round_trip() {
        let req = IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: "Zone Temp".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = IHaveRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_who_has_empty_input() {
        assert!(WhoHasRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_who_has_truncated_1_byte() {
        let req = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Identifier(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            ),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(WhoHasRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_who_has_truncated_2_bytes() {
        let req = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Identifier(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            ),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(WhoHasRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_who_has_invalid_tag() {
        // Context tag that is neither 0, 1, 2, nor 3
        assert!(WhoHasRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_who_has_oversized_length() {
        assert!(WhoHasRequest::decode(&[0x05, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_i_have_empty_input() {
        assert!(IHaveRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_i_have_truncated_1_byte() {
        let req = IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: "Zone Temp".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(IHaveRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_i_have_truncated_2_bytes() {
        let req = IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: "Zone Temp".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(IHaveRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_i_have_truncated_3_bytes() {
        let req = IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: "Zone Temp".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(IHaveRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_i_have_truncated_half() {
        let req = IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: "Zone Temp".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(IHaveRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_i_have_invalid_tag() {
        assert!(IHaveRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
