//! WriteProperty service per ASHRAE 135-2020 Clause 15.9.

use bacnet_encoding::primitives;
use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// WritePropertyRequest (Clause 15.9.1.1)
// ---------------------------------------------------------------------------

/// WriteProperty-Request service parameters.
///
/// ```text
/// WriteProperty-Request ::= SEQUENCE {
///     objectIdentifier    [0] BACnetObjectIdentifier,
///     propertyIdentifier  [1] BACnetPropertyIdentifier,
///     propertyArrayIndex  [2] Unsigned OPTIONAL,
///     propertyValue       [3] ABSTRACT-SYNTAX.&TYPE,
///     priority            [4] Unsigned (1..16) OPTIONAL
/// }
/// ```
///
/// WriteProperty uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyRequest {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    pub property_value: Vec<u8>,
    pub priority: Option<u8>,
}

impl WritePropertyRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        primitives::encode_ctx_unsigned(buf, 1, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
        tags::encode_opening_tag(buf, 3);
        buf.extend_from_slice(&self.property_value);
        tags::encode_closing_tag(buf, 3);
        if let Some(prio) = self.priority {
            primitives::encode_ctx_unsigned(buf, 4, prio as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "WriteProperty truncated at object-id"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] property-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "WriteProperty truncated at property-id",
            ));
        }
        let prop_raw = primitives::decode_unsigned(&data[pos..end])? as u32;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);
        offset = end;

        // [2] propertyArrayIndex (optional) — peek for context tag 2
        let mut property_array_index = None;
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if tag.class == TagClass::Context && tag.number == 2 && !tag.is_opening && !tag.is_closing {
            let end = tag_end + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    tag_end,
                    "WriteProperty truncated at array-index",
                ));
            }
            property_array_index = Some(primitives::decode_unsigned(&data[tag_end..end])? as u32);
            offset = end;
        }

        // [3] propertyValue (opening/closing tag 3)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(3) {
            return Err(Error::decoding(
                offset,
                "WriteProperty expected opening tag 3",
            ));
        }
        let (value_bytes, new_offset) = tags::extract_context_value(data, tag_end, 3)?;
        let property_value = value_bytes.to_vec();
        offset = new_offset;

        // [4] priority (optional)
        let mut priority = None;
        if offset < data.len() {
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(4) {
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(pos, "WriteProperty truncated at priority"));
                }
                let prio = primitives::decode_unsigned(&data[pos..end])? as u8;
                if !(1..=16).contains(&prio) {
                    return Err(Error::decoding(
                        pos,
                        format!("WriteProperty priority {prio} out of range 1-16"),
                    ));
                }
                priority = Some(prio);
            }
        }

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn request_round_trip() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_with_all_fields() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(5),
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn priority_validation() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x91, 0x01], // enumerated 1 (active)
            priority: Some(16),               // max valid
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(decoded.priority, Some(16));
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_write_property_empty_input() {
        assert!(WritePropertyRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_1_byte() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_2_bytes() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_3_bytes() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_half() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(WritePropertyRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_write_property_invalid_tag() {
        assert!(WritePropertyRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_write_property_oversized_length() {
        // Tag with oversized length field
        assert!(WritePropertyRequest::decode(&[0x05, 0xFF]).is_err());
    }
}
