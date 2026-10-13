//! ReadProperty service per ASHRAE 135-2020 Clause 15.5.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx, expect_end, expect_opening,
};
use bacnet_encoding::constructed::{extract_property_value, PropertyValueBoundary};

// ---------------------------------------------------------------------------
// ReadPropertyRequest
// ---------------------------------------------------------------------------

/// ReadProperty-Request service parameters (Clause 15.5; production in Clause 21.2).
///
/// On the wire: the object identifier in context tag `[0]`, the property identifier in `[1]`,
/// then an optional Unsigned array index in `[2]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyRequest {
    /// Object to read from.
    pub object_identifier: ObjectIdentifier,
    /// Property to read.
    pub property_identifier: PropertyIdentifier,
    /// Array index of the element to read; `None` reads the whole property, and index 0 requests
    /// the array length.
    pub property_array_index: Option<u32>,
}

impl ReadPropertyRequest {
    /// Encode the request parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        primitives::encode_ctx_unsigned(buf, 1, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] object-identifier
        let (object_identifier, offset) =
            decode_ctx_object_id(data, 0, 0, "ReadProperty request object-id")?;

        // [1] property-identifier
        let (prop_raw, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "ReadProperty request property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);

        // [2] propertyArrayIndex (optional), and nothing after it
        let (property_array_index, end) = decode_optional_ctx(
            data,
            offset,
            2,
            "ReadProperty request array-index",
            decode_ctx_unsigned::<u32>,
        )?;
        expect_end(data, end, end, "ReadProperty request")?;

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
        })
    }
}

// ---------------------------------------------------------------------------
// ReadPropertyACK
// ---------------------------------------------------------------------------

/// ReadProperty-ACK service parameters (Clause 15.5; production in Clause 21.2).
///
/// The ACK repeats the request's members at the same context tags (`[0]` object, `[1]`
/// property, optional `[2]` array index) and adds the value inside an opening/closing `[3]`
/// pair, typed by whatever the property holds.
///
/// The `property_value` field contains raw application-tagged bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyACK {
    /// Object that was read.
    pub object_identifier: ObjectIdentifier,
    /// Property that was read.
    pub property_identifier: PropertyIdentifier,
    /// Array index that was requested, echoed from the request; `None` when the whole property was
    /// read.
    pub property_array_index: Option<u32>,
    /// Application-tagged encoding of the value, opaque to this crate.
    pub property_value: Vec<u8>,
}

impl ReadPropertyACK {
    /// Encode the acknowledgment parameters into `buf`.
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

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] object-identifier
        let (object_identifier, offset) =
            decode_ctx_object_id(data, 0, 0, "ReadPropertyACK object-id")?;

        // [1] property-identifier
        let (prop_raw, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "ReadPropertyACK property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);

        // [2] propertyArrayIndex (optional)
        let (property_array_index, offset) = decode_optional_ctx(
            data,
            offset,
            2,
            "ReadPropertyACK array-index",
            decode_ctx_unsigned::<u32>,
        )?;

        // [3] propertyValue, and nothing after it
        let content = expect_opening(data, offset, 3, "ReadPropertyACK property-value")?;
        let (value_bytes, end) = extract_property_value(
            data,
            content,
            3,
            property_identifier,
            &[PropertyValueBoundary::End],
        )?;
        if end != data.len() {
            return Err(Error::decoding(end, "ReadPropertyACK has trailing data"));
        }

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value: value_bytes.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::tags::TagClass;
    use bacnet_types::enums::ObjectType;

    fn object_id() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
    }

    fn encode_context_value(buf: &mut BytesMut, tag_number: u8, value: u64, leading_zero: bool) {
        if leading_zero {
            tags::encode_tag(buf, tag_number, TagClass::Context, 5);
            buf.extend_from_slice(&value.to_be_bytes()[3..]);
        } else {
            primitives::encode_ctx_unsigned(buf, tag_number, value);
        }
    }

    fn encode_fields(
        object_tag: u8,
        property_tag: u8,
        property: u64,
        index: Option<(u8, u64)>,
        ack: bool,
        leading_zero: bool,
    ) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_ctx_object_id(&mut buf, object_tag, &object_id());
        encode_context_value(&mut buf, property_tag, property, leading_zero);
        if let Some((tag_number, value)) = index {
            encode_context_value(&mut buf, tag_number, value, leading_zero);
        }
        if ack {
            tags::encode_opening_tag(&mut buf, 3);
            primitives::encode_app_null(&mut buf);
            tags::encode_closing_tag(&mut buf, 3);
        }
        buf
    }

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

    #[test]
    fn legacy_event_parameters_cross_property_framing() {
        use crate::write_property::WritePropertyRequest;
        use bacnet_types::constructed::BACnetEventParameter;

        let value = BACnetEventParameter::Opaque {
            tag: u8::MAX,
            data: vec![0xff, 1, 2],
        };
        let mut property_value = BytesMut::new();
        bacnet_encoding::constructed::encode_event_parameter(&mut property_value, &value).unwrap();
        let object_identifier = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();

        let ack = ReadPropertyACK {
            object_identifier,
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            property_value: property_value.to_vec(),
        };
        let mut encoded = BytesMut::new();
        ack.encode(&mut encoded);
        let decoded = ReadPropertyACK::decode(&encoded).unwrap();
        assert_eq!(decoded, ack);

        let request = WritePropertyRequest {
            object_identifier,
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            property_value: property_value.to_vec(),
            priority: None,
        };
        encoded.clear();
        request.encode(&mut encoded).unwrap();
        let decoded = WritePropertyRequest::decode(&encoded).unwrap();
        assert_eq!(decoded, request);

        let (decoded, end) =
            bacnet_encoding::constructed::decode_event_parameter(&property_value, 0).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(end, property_value.len());
    }

    #[test]
    fn historical_event_parameters_cross_property_framing() {
        use crate::write_property::WritePropertyRequest;

        let property_value = vec![0xfe, 0xff, 1, 0xff, 0xff, 0x3f, 0x49, 8, 2, 0xff, 0xff];
        let object_identifier = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
        let ack = ReadPropertyACK {
            object_identifier,
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            property_value: property_value.clone(),
        };
        let mut encoded = BytesMut::new();
        ack.encode(&mut encoded);
        assert_eq!(ReadPropertyACK::decode(&encoded).unwrap(), ack);

        let request = WritePropertyRequest {
            object_identifier,
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            property_value,
            priority: Some(8),
        };
        encoded.clear();
        request.encode(&mut encoded).unwrap();
        assert_eq!(WritePropertyRequest::decode(&encoded).unwrap(), request);
    }

    #[test]
    fn read_property_values_must_fit_u32() {
        let maximum = u64::from(u32::MAX);
        let request = encode_fields(0, 1, maximum, Some((2, maximum)), false, true);
        let request = ReadPropertyRequest::decode(&request).unwrap();
        assert_eq!(request.property_identifier.to_raw(), u32::MAX);
        assert_eq!(request.property_array_index, Some(u32::MAX));

        let ack = encode_fields(0, 1, maximum, Some((2, maximum)), true, true);
        let ack = ReadPropertyACK::decode(&ack).unwrap();
        assert_eq!(ack.property_identifier.to_raw(), u32::MAX);
        assert_eq!(ack.property_array_index, Some(u32::MAX));

        for value in [maximum + 1, u64::MAX] {
            let request_property = encode_fields(0, 1, value, None, false, false);
            assert!(ReadPropertyRequest::decode(&request_property).is_err());
            let request_index = encode_fields(0, 1, 1, Some((2, value)), false, false);
            assert!(ReadPropertyRequest::decode(&request_index).is_err());

            let ack_property = encode_fields(0, 1, value, None, true, false);
            assert!(ReadPropertyACK::decode(&ack_property).is_err());
            let ack_index = encode_fields(0, 1, 1, Some((2, value)), true, false);
            assert!(ReadPropertyACK::decode(&ack_index).is_err());
        }
    }

    #[test]
    fn read_property_requires_mandatory_context_tags() {
        let wrong_object = encode_fields(1, 1, 1, None, false, false);
        assert!(ReadPropertyRequest::decode(&wrong_object).is_err());
        let wrong_object_ack = encode_fields(1, 1, 1, None, true, false);
        assert!(ReadPropertyACK::decode(&wrong_object_ack).is_err());

        let wrong_property = encode_fields(0, 0, 1, None, false, false);
        assert!(ReadPropertyRequest::decode(&wrong_property).is_err());
        let wrong_property_ack = encode_fields(0, 0, 1, None, true, false);
        assert!(ReadPropertyACK::decode(&wrong_property_ack).is_err());

        let mut application_property = BytesMut::new();
        primitives::encode_ctx_object_id(&mut application_property, 0, &object_id());
        primitives::encode_app_unsigned(&mut application_property, 1);
        assert!(ReadPropertyRequest::decode(&application_property).is_err());
    }

    #[test]
    fn read_property_rejects_malformed_optional_and_trailing_fields() {
        let request = ReadPropertyRequest {
            object_identifier: object_id(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut request_data = BytesMut::new();
        request.encode(&mut request_data);
        request_data.extend_from_slice(&[0x2e]);
        assert!(ReadPropertyRequest::decode(&request_data).is_err());

        let mut indexed_request = encode_fields(0, 1, 1, Some((2, 1)), false, false);
        primitives::encode_ctx_unsigned(&mut indexed_request, 4, 1);
        assert!(ReadPropertyRequest::decode(&indexed_request).is_err());

        let mut ack = encode_fields(0, 1, 1, None, true, false);
        primitives::encode_ctx_unsigned(&mut ack, 4, 1);
        assert!(ReadPropertyACK::decode(&ack).is_err());

        let mut indexed_ack = encode_fields(0, 1, 1, Some((2, 1)), true, false);
        primitives::encode_ctx_unsigned(&mut indexed_ack, 4, 1);
        assert!(ReadPropertyACK::decode(&indexed_ack).is_err());
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
