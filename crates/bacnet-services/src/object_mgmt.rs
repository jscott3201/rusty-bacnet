//! Object management services per ASHRAE 135-2020 Clause 15.3-15.4.
//!
//! - CreateObject (Clause 15.3)
//! - DeleteObject (Clause 15.4)

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_app_object_id, decode_ctx_object_id, decode_ctx_unsigned, expect_closing, expect_end,
    expect_opening, misplaced_tag, next_is_context, next_is_opening,
};
use bacnet_encoding::constructed::{
    decode_bacnet_property_value_in_list, encode_bacnet_property_value,
};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};

mod error;
pub use error::CreateObjectError;

// ---------------------------------------------------------------------------
// CreateObjectRequest
// ---------------------------------------------------------------------------

/// The object specifier: by type (server picks instance) or by identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectSpecifier {
    /// Create by type — server assigns instance number (\[0\] context tag inside \[0\] constructed).
    Type(ObjectType),
    /// Create with a specific identifier (\[1\] context tag inside \[0\] constructed).
    Identifier(ObjectIdentifier),
}

/// CreateObject-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateObjectRequest {
    /// Object to create: either a type, letting the server choose the instance, or a full
    /// identifier.
    pub object_specifier: ObjectSpecifier,
    /// Initial property values to apply to the new object; empty means none were supplied.
    pub list_of_initial_values: Vec<BACnetPropertyValue>,
}

impl CreateObjectRequest {
    /// Encode the request parameters into `buf`.
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
                encode_bacnet_property_value(pv, buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] object-specifier, a CHOICE read in place so a member cut short
        // is reported as such
        let what = "CreateObject object-specifier";
        let offset = expect_opening(data, 0, 0, what)?;
        let (object_specifier, offset) = if next_is_context(data, offset, 0)? {
            let (raw, end) =
                decode_ctx_unsigned::<u32>(data, offset, 0, "CreateObject object-type")?;
            (ObjectSpecifier::Type(ObjectType::from_raw(raw)), end)
        } else if next_is_context(data, offset, 1)? {
            let (oid, end) =
                decode_ctx_object_id(data, offset, 1, "CreateObject object-identifier")?;
            (ObjectSpecifier::Identifier(oid), end)
        } else {
            // Neither alternative: the frame closes empty, or another tag
            // stands there.
            let (found, _) = tags::decode_tag(data, offset)?;
            return Err(misplaced_tag(
                data,
                &found,
                None,
                offset,
                "CreateObject expected context tag 0 or 1 inside object-specifier",
            ));
        };
        let mut offset = expect_closing(data, offset, 0, what)?;

        // [1] list-of-initial-values (optional, opening tag 1)
        let mut values = Vec::new();
        if next_is_opening(data, offset, 1)? {
            offset = expect_opening(data, offset, 1, "CreateObject list-of-initial-values")?;
            loop {
                if offset >= data.len() {
                    return Err(Error::missing(offset, "CreateObject missing closing tag 1"));
                }
                let (tag, closing_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = closing_end;
                    break;
                }
                if values.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::overflow(offset, "CreateObject values exceeds max"));
                }
                let (pv, new_offset) = decode_bacnet_property_value_in_list(data, offset, 1)?;
                values.push(pv);
                offset = new_offset;
            }
        }

        expect_end(data, offset, offset, "CreateObject")?;

        Ok(Self {
            object_specifier,
            list_of_initial_values: values,
        })
    }
}

// ---------------------------------------------------------------------------
// CreateObjectAck
// ---------------------------------------------------------------------------

/// CreateObject-ACK service parameters, the Result(+) of Clause 15.3.1.2 as
/// Clause 21 encodes it: the identifier of the object the server created,
/// application-tagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateObjectAck {
    /// Identifier of the newly created object.
    pub object_identifier: ObjectIdentifier,
}

impl CreateObjectAck {
    /// Encode the acknowledgment parameter into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.object_identifier);
    }

    /// Decode the acknowledgment from service-ack octets; fails on malformed or truncated
    /// input, on an identifier under any tag but its application tag, and on octets after it.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (object_identifier, end) =
            decode_app_object_id(data, 0, "CreateObject-ACK object-identifier")?;
        expect_end(data, end, end, "CreateObject-ACK")?;
        Ok(Self { object_identifier })
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
    /// Object to delete.
    pub object_identifier: ObjectIdentifier,
}

impl DeleteObjectRequest {
    /// Encode the request parameter into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.object_identifier);
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on an object identifier under any tag but its application tag, and on octets after it.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (object_identifier, end) =
            decode_app_object_id(data, 0, "DeleteObject object-identifier")?;
        expect_end(data, end, end, "DeleteObject")?;
        Ok(Self { object_identifier })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    fn create_object_type_bytes(object_type: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        tags::encode_opening_tag(&mut buf, 0);
        primitives::encode_ctx_octet_string(&mut buf, 0, object_type);
        tags::encode_closing_tag(&mut buf, 0);
        buf
    }

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
    fn create_object_type_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];
        let decoded =
            CreateObjectRequest::decode(&create_object_type_bytes(&max_with_leading_zero)).unwrap();
        assert_eq!(
            decoded.object_specifier,
            ObjectSpecifier::Type(ObjectType::from_raw(u32::MAX))
        );

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            assert!(CreateObjectRequest::decode(&create_object_type_bytes(
                &overflow.to_be_bytes(),
            ))
            .is_err());
        }
    }

    #[test]
    fn create_object_rejects_unexpected_or_trailing_fields() {
        let encoded = create_object_type_bytes(&[0]);

        let mut empty_values = encoded.clone();
        tags::encode_opening_tag(&mut empty_values, 1);
        tags::encode_closing_tag(&mut empty_values, 1);
        assert!(CreateObjectRequest::decode(&empty_values).is_ok());

        let mut unexpected = encoded.clone();
        primitives::encode_ctx_unsigned(&mut unexpected, 1, 1);
        assert!(CreateObjectRequest::decode(&unexpected).is_err());

        let mut trailing = empty_values;
        primitives::encode_app_null(&mut trailing);
        assert!(CreateObjectRequest::decode(&trailing).is_err());
    }

    #[test]
    fn create_object_accepts_exact_initial_value_limit() {
        let value = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0],
            priority: None,
        };
        let request = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![value.clone(); MAX_DECODED_ITEMS],
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded);
        assert_eq!(
            CreateObjectRequest::decode(&encoded)
                .unwrap()
                .list_of_initial_values
                .len(),
            MAX_DECODED_ITEMS
        );

        let request = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![value; MAX_DECODED_ITEMS + 1],
        };
        encoded.clear();
        request.encode(&mut encoded);
        assert!(CreateObjectRequest::decode(&encoded).is_err());
    }

    #[test]
    fn create_object_ack_round_trip_is_one_application_object_id() {
        let ack = CreateObjectAck {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        // Application tag 12 (object identifier), length 4, then the packed id.
        assert_eq!(&buf[..], &[0xC4, 0x00, 0x80, 0x00, 0x07]);
        assert_eq!(CreateObjectAck::decode(&buf).unwrap(), ack);
    }

    #[test]
    fn create_object_ack_rejects_malformed_input() {
        let ack = CreateObjectAck {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(CreateObjectAck::decode(&[]).is_err());
        for cut in 1..buf.len() {
            assert!(CreateObjectAck::decode(&buf[..cut]).is_err(), "cut {cut}");
        }
        // Trailing octet after the identifier.
        let mut trailing = buf.to_vec();
        trailing.push(0x00);
        assert!(CreateObjectAck::decode(&trailing).is_err());
        // Context tag 0 instead of the application tag.
        let mut context = BytesMut::new();
        primitives::encode_ctx_object_id(&mut context, 0, &ack.object_identifier);
        assert!(CreateObjectAck::decode(&context).is_err());
        // An application-tagged unsigned is not an object identifier.
        assert!(CreateObjectAck::decode(&[0x21, 0x07]).is_err());
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
