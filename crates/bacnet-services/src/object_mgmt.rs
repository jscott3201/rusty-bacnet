//! Object management services per ASHRAE 135-2020 Clause 15.3-15.4.
//!
//! - CreateObject (Clause 15.3)
//! - DeleteObject (Clause 15.4)

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};

// ---------------------------------------------------------------------------
// CreateObjectRequest
// ---------------------------------------------------------------------------

/// The object specifier: by type (server picks instance) or by identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectSpecifier {
    /// Create by type — server assigns instance number ([0] context tag inside [0] constructed).
    Type(ObjectType),
    /// Create with a specific identifier ([1] context tag inside [0] constructed).
    Identifier(ObjectIdentifier),
}

/// CreateObject-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateObjectRequest {
    pub object_specifier: ObjectSpecifier,
    pub list_of_initial_values: Vec<BACnetPropertyValue>,
}

impl CreateObjectRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] object-specifier (constructed)
        tags::encode_opening_tag(buf, 0);
        match &self.object_specifier {
            ObjectSpecifier::Type(obj_type) => {
                primitives::encode_ctx_enumerated(buf, 0, obj_type.to_raw());
            }
            ObjectSpecifier::Identifier(oid) => {
                primitives::encode_ctx_object_id(buf, 1, oid);
            }
        }
        tags::encode_closing_tag(buf, 0);

        // [1] list-of-initial-values (optional, constructed)
        if !self.list_of_initial_values.is_empty() {
            tags::encode_opening_tag(buf, 1);
            for pv in &self.list_of_initial_values {
                pv.encode(buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] object-specifier (opening tag 0)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(0) {
            return Err(Error::decoding(
                offset,
                "CreateObject expected opening tag 0",
            ));
        }
        offset = tag_end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "CreateObject truncated at object-specifier",
            ));
        }

        let object_specifier = if tag.is_context(0) {
            let raw = primitives::decode_unsigned(&data[pos..end])? as u32;
            ObjectSpecifier::Type(ObjectType::from_raw(raw))
        } else if tag.is_context(1) {
            ObjectSpecifier::Identifier(ObjectIdentifier::decode(&data[pos..end])?)
        } else {
            return Err(Error::decoding(
                offset,
                "CreateObject expected context tag 0 or 1 inside object-specifier",
            ));
        };
        offset = end;

        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_closing_tag(0) {
            return Err(Error::decoding(
                offset,
                "CreateObject expected closing tag 0",
            ));
        }
        offset = tag_end;

        // [1] list-of-initial-values (optional, opening tag 1)
        let mut values = Vec::new();
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(1) {
                offset = tag_end;
                loop {
                    if offset >= data.len() {
                        return Err(Error::decoding(
                            offset,
                            "CreateObject missing closing tag 1",
                        ));
                    }
                    if values.len() >= MAX_DECODED_ITEMS {
                        return Err(Error::decoding(offset, "CreateObject values exceeds max"));
                    }
                    let (tag, _tag_end) = tags::decode_tag(data, offset)?;
                    if tag.is_closing_tag(1) {
                        break;
                    }
                    let (pv, new_offset) = BACnetPropertyValue::decode(data, offset)?;
                    values.push(pv);
                    offset = new_offset;
                }
            }
        }

        Ok(Self {
            object_specifier,
            list_of_initial_values: values,
        })
    }
}

// ---------------------------------------------------------------------------
// DeleteObjectRequest
// ---------------------------------------------------------------------------

/// DeleteObject-Request service parameters (APPLICATION-tagged).
///
/// Uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteObjectRequest {
    pub object_identifier: ObjectIdentifier,
}

impl DeleteObjectRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.object_identifier);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, pos) = tags::decode_tag(data, 0)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "DeleteObject truncated at object-id"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        Ok(Self { object_identifier })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    #[test]
    fn create_object_by_type_round_trip() {
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = CreateObjectRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn create_object_by_id_with_values() {
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            ),
            list_of_initial_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
                value: vec![0x75, 0x06, 0x00, 0x5A, 0x6F, 0x6E, 0x65, 0x31],
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = CreateObjectRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn delete_object_round_trip() {
        let req = DeleteObjectRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = DeleteObjectRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_create_object_empty_input() {
        assert!(CreateObjectRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_create_object_truncated_1_byte() {
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(CreateObjectRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_create_object_truncated_2_bytes() {
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(CreateObjectRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_create_object_truncated_3_bytes() {
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        if buf.len() > 3 {
            assert!(CreateObjectRequest::decode(&buf[..3]).is_err());
        }
    }

    #[test]
    fn test_decode_create_object_invalid_tag() {
        // First byte should be opening tag 0, not 0xFF
        assert!(CreateObjectRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_create_object_missing_closing_tag() {
        // Opening tag 0 (0x0E) but no closing tag
        assert!(CreateObjectRequest::decode(&[0x0E, 0x09, 0x00]).is_err());
    }

    #[test]
    fn test_decode_delete_object_empty_input() {
        assert!(DeleteObjectRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_delete_object_truncated_1_byte() {
        let req = DeleteObjectRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(DeleteObjectRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_delete_object_truncated_2_bytes() {
        let req = DeleteObjectRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(DeleteObjectRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_delete_object_invalid_tag() {
        assert!(DeleteObjectRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
