//! Who-Has and I-Have services per ASHRAE 135-2020 Clause 16.9.

#[cfg(not(feature = "std"))]
use alloc::string::String;

use crate::who_is::{DeviceInstanceRange, LimitLabels, WireLimits};
use bacnet_encoding::constructed::tagged::{
    decode_app_character_string, decode_app_object_id, decode_ctx_character_string,
    decode_ctx_object_id, expect_end, next_is_context,
};
use bacnet_encoding::primitives;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// WhoHasRequest
// ---------------------------------------------------------------------------

/// The object to search for: by identifier or by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhoHasObject {
    /// Search by object identifier (\[2\] context tag).
    Identifier(ObjectIdentifier),
    /// Search by object name (\[3\] context tag).
    Name(String),
}

/// Who-Has-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoHasRequest {
    /// The devices that should answer; `None` asks every device.
    pub range: Option<DeviceInstanceRange>,
    /// Object being searched for, by identifier or by name.
    pub object: WhoHasObject,
}

impl WhoHasRequest {
    /// Encode the request parameters into `buf`; fails if the object name cannot be encoded.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] low limit and [1] high limit, both or neither
        if let Some(range) = self.range {
            range.encode(buf);
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

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on octets after the object, and, as a Who-Is does, on one limit without the other or a
    /// low limit above the high one (#1483). The request is unconfirmed, so a receiver drops
    /// it unanswered rather than reading one limit as a request for every device.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] low limit and [1] high limit, both or neither
        let limits = WireLimits::decode(data, &LimitLabels::WHO_HAS)?;
        let offset = limits.end;

        // CHOICE: [2] object-identifier OR [3] object-name
        let (object, end) = if next_is_context(data, offset, 2)? {
            let (oid, end) = decode_ctx_object_id(data, offset, 2, "WhoHas object-identifier")?;
            (WhoHasObject::Identifier(oid), end)
        } else if next_is_context(data, offset, 3)? {
            let (name, end) = decode_ctx_character_string(data, offset, 3, "WhoHas object-name")?;
            (WhoHasObject::Name(name), end)
        } else {
            return Err(Error::decoding(
                offset,
                "WhoHas expected context tag 2 or 3",
            ));
        };
        expect_end(data, end, end, "WhoHas")?;

        Ok(Self {
            range: limits.range()?,
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
    /// Device object of the responding device.
    pub device_identifier: ObjectIdentifier,
    /// Identifier of the object that was found.
    pub object_identifier: ObjectIdentifier,
    /// Name of the object that was found.
    pub object_name: String,
}

impl IHaveRequest {
    /// Encode the request parameters into `buf`; fails if the object name cannot be encoded.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        primitives::encode_app_object_id(buf, &self.device_identifier);
        primitives::encode_app_object_id(buf, &self.object_identifier);
        primitives::encode_app_character_string(buf, &self.object_name)?;
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on a member under any tag but its application tag, and on octets after the name.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (device_identifier, offset) = decode_app_object_id(data, 0, "IHave device-identifier")?;
        let (object_identifier, offset) =
            decode_app_object_id(data, offset, "IHave object-identifier")?;
        let (object_name, end) = decode_app_character_string(data, offset, "IHave object-name")?;
        expect_end(data, end, end, "IHave")?;

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
    use bacnet_encoding::tags;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn who_has_by_id_round_trip() {
        let req = WhoHasRequest {
            range: None,
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
            range: Some(DeviceInstanceRange::new(1000, 2000).unwrap()),
            object: WhoHasObject::Name("Zone Temperature".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = WhoHasRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn who_has_limits_must_fit_u32() {
        let object_identifier = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let encode_request = |low, high| {
            let mut buf = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut buf, 0, low);
            primitives::encode_ctx_unsigned(&mut buf, 1, high);
            primitives::encode_ctx_object_id(&mut buf, 2, &object_identifier);
            buf
        };

        // The shared readers' wording names the member, its tag and the value.
        for (low, high, field, value) in [
            (
                4_294_967_296,
                4_294_967_296,
                "low limit: [0]",
                4_294_967_296_u64,
            ),
            (1, 4_294_967_297, "high limit: [1]", 4_294_967_297),
            (u64::MAX, u64::MAX, "low limit: [0]", u64::MAX),
        ] {
            let encoded = encode_request(low, high);
            let error = WhoHasRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("WhoHas {field} value {value} exceeds u32")),
                "unexpected error for {field} {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        for tag_number in [0, 1] {
            tags::encode_tag(&mut leading_zero, tag_number, tags::TagClass::Context, 5);
            leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        }
        primitives::encode_ctx_object_id(&mut leading_zero, 2, &object_identifier);
        let decoded = WhoHasRequest::decode(&leading_zero).unwrap();
        // Past the highest instance, but taken as written.
        let range = decoded.range.unwrap();
        assert_eq!((range.low(), range.high()), (u32::MAX, u32::MAX));
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
            range: None,
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
            range: None,
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
