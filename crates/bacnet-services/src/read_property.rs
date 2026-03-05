//! ReadProperty service per ASHRAE 135-2020 Clause 15.5.

use bacnet_encoding::primitives;
use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// ReadPropertyRequest (Clause 15.5.1.1)
// ---------------------------------------------------------------------------

/// ReadProperty-Request service parameters.
///
/// ```text
/// ReadProperty-Request ::= SEQUENCE {
///     objectIdentifier    [0] BACnetObjectIdentifier,
///     propertyIdentifier  [1] BACnetPropertyIdentifier,
///     propertyArrayIndex  [2] Unsigned OPTIONAL
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyRequest {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
}

impl ReadPropertyRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        primitives::encode_ctx_unsigned(buf, 1, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadProperty request truncated at object-id",
            ));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] property-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadProperty request truncated at property-id",
            ));
        }
        let prop_raw = primitives::decode_unsigned(&data[pos..end])? as u32;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        if offset < data.len() {
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(2) {
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        pos,
                        "ReadProperty request truncated at array-index",
                    ));
                }
                property_array_index = Some(primitives::decode_unsigned(&data[pos..end])? as u32);
            }
        }

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
        })
    }
}

// ---------------------------------------------------------------------------
// ReadPropertyACK (Clause 15.5.1.2)
// ---------------------------------------------------------------------------

/// ReadProperty-ACK service parameters.
///
/// ```text
/// ReadProperty-ACK ::= SEQUENCE {
///     objectIdentifier    [0] BACnetObjectIdentifier,
///     propertyIdentifier  [1] BACnetPropertyIdentifier,
///     propertyArrayIndex  [2] Unsigned OPTIONAL,
///     propertyValue       [3] ABSTRACT-SYNTAX.&TYPE
/// }
/// ```
///
/// The `property_value` field contains raw application-tagged bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyACK {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    pub property_value: Vec<u8>,
}

impl ReadPropertyACK {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        primitives::encode_ctx_unsigned(buf, 1, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
        tags::encode_opening_tag(buf, 3);
        buf.extend_from_slice(&self.property_value);
        tags::encode_closing_tag(buf, 3);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadPropertyACK truncated at object-id",
            ));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] property-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadPropertyACK truncated at property-id",
            ));
        }
        let prop_raw = primitives::decode_unsigned(&data[pos..end])? as u32;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);
        offset = end;

        // [2] propertyArrayIndex (optional) or [3] opening tag
        let mut property_array_index = None;
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if tag.class == TagClass::Context && tag.number == 2 && !tag.is_opening && !tag.is_closing {
            let end = tag_end + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    tag_end,
                    "ReadPropertyACK truncated at array-index",
                ));
            }
            property_array_index = Some(primitives::decode_unsigned(&data[tag_end..end])? as u32);
            offset = end;
            // Read opening tag 3
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(3) {
                return Err(Error::decoding(
                    offset,
                    "ReadPropertyACK expected opening tag 3",
                ));
            }
            let (value_bytes, _end) = tags::extract_context_value(data, tag_end, 3)?;
            return Ok(Self {
                object_identifier,
                property_identifier,
                property_array_index,
                property_value: value_bytes.to_vec(),
            });
        }

        // tag should be opening tag 3
        if !tag.is_opening_tag(3) {
            return Err(Error::decoding(
                offset,
                "ReadPropertyACK expected opening tag 3",
            ));
        }
        let (value_bytes, _) = tags::extract_context_value(data, tag_end, 3)?;

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value: value_bytes.to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn request_round_trip() {
        let req = ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadPropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_with_index_round_trip() {
        let req = ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 5).unwrap(),
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: Some(8),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadPropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn ack_round_trip() {
        let ack = ReadPropertyACK {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00], // Real 72.5
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadPropertyACK::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_with_index_round_trip() {
        let ack = ReadPropertyACK {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: Some(8),
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadPropertyACK::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_read_property_request_empty_input() {
        assert!(ReadPropertyRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_read_property_request_truncated_1_byte() {
        // Encode a valid request, then truncate to 1 byte
        let req = ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadPropertyRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_read_property_request_truncated_2_bytes() {
        let req = ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadPropertyRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_read_property_request_truncated_3_bytes() {
        let req = ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadPropertyRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_read_property_request_invalid_tag() {
        // 0xFF is not a valid starting tag byte in BACnet context
        assert!(ReadPropertyRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_read_property_request_oversized_length() {
        // Tag byte claiming a length that exceeds available data
        // Context tag 0, extended length indicator (5 = len in next byte), then huge length
        assert!(ReadPropertyRequest::decode(&[0x05, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_read_property_ack_empty_input() {
        assert!(ReadPropertyACK::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_read_property_ack_truncated_1_byte() {
        let ack = ReadPropertyACK {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadPropertyACK::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_read_property_ack_truncated_3_bytes() {
        let ack = ReadPropertyACK {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadPropertyACK::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_read_property_ack_truncated_half() {
        let ack = ReadPropertyACK {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(ReadPropertyACK::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_read_property_ack_invalid_tag() {
        assert!(ReadPropertyACK::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
