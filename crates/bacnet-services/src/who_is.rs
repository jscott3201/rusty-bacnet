//! Who-Is and I-Am services per ASHRAE 135-2020 Clause 16.10.

use bacnet_encoding::primitives;
use bacnet_encoding::tags::{self};
use bacnet_types::enums::Segmentation;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// WhoIsRequest
// ---------------------------------------------------------------------------

/// Who-Is-Request service parameters.
///
/// Both limits must be present or both absent. If only one is set,
/// the request is treated as unbounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoIsRequest {
    pub low_limit: Option<u32>,
    pub high_limit: Option<u32>,
}

impl WhoIsRequest {
    /// Create an unbounded WhoIs (all devices).
    pub fn all() -> Self {
        Self {
            low_limit: None,
            high_limit: None,
        }
    }

    /// Create a ranged WhoIs.
    pub fn range(low: u32, high: u32) -> Self {
        Self {
            low_limit: Some(low),
            high_limit: Some(high),
        }
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        if let (Some(low), Some(high)) = (self.low_limit, self.high_limit) {
            primitives::encode_ctx_unsigned(buf, 0, low as u64);
            primitives::encode_ctx_unsigned(buf, 1, high as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.is_empty() {
            return Ok(Self::all());
        }

        let mut offset = 0;
        let mut low_limit = None;
        let mut high_limit = None;

        // [0] device-instance-range-low-limit
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.is_context(0) {
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "WhoIs truncated at low-limit"));
            }
            let low_limit_raw = primitives::decode_unsigned(&data[pos..end])?;
            low_limit = Some(u32::try_from(low_limit_raw).map_err(|_| {
                Error::decoding(pos, format!("WhoIs low-limit {low_limit_raw} exceeds u32"))
            })?);
            offset = end;
        }

        // [1] device-instance-range-high-limit
        if offset < data.len() {
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(pos, "WhoIs truncated at high-limit"));
                }
                let high_limit_raw = primitives::decode_unsigned(&data[pos..end])?;
                high_limit = Some(u32::try_from(high_limit_raw).map_err(|_| {
                    Error::decoding(
                        pos,
                        format!("WhoIs high-limit {high_limit_raw} exceeds u32"),
                    )
                })?);
            }
        }

        // Both present or both absent
        if low_limit.is_some() != high_limit.is_some() {
            tracing::warn!("WhoIs: only one of low/high limit present — treating as unbounded per lenient decode policy");
            return Ok(Self::all());
        }

        if let (Some(low), Some(high)) = (low_limit, high_limit) {
            if low > high {
                return Err(Error::decoding(0, "WhoIs low_limit exceeds high_limit"));
            }
        }

        Ok(Self {
            low_limit,
            high_limit,
        })
    }
}

// ---------------------------------------------------------------------------
// IAmRequest
// ---------------------------------------------------------------------------

/// I-Am-Request service parameters.
///
/// All fields use APPLICATION tags (not context-specific).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IAmRequest {
    pub object_identifier: ObjectIdentifier,
    pub max_apdu_length: u32,
    pub segmentation_supported: Segmentation,
    pub vendor_id: u16,
}

impl IAmRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.object_identifier);
        primitives::encode_app_unsigned(buf, self.max_apdu_length as u64);
        primitives::encode_app_enumerated(buf, self.segmentation_supported.to_raw() as u32);
        primitives::encode_app_unsigned(buf, self.vendor_id as u64);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        // Application L/V/T values 6 and 7 are reserved, but decode_tag treats
        // them as extended lengths because they mark context opening/closing tags.
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::OBJECT_IDENTIFIER
            || data[offset] & 0x07 > 5
        {
            return Err(Error::decoding(
                offset,
                "IAm object identifier: expected application-tagged object identifier",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at object-identifier"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::UNSIGNED
            || data[offset] & 0x07 > 5
        {
            return Err(Error::decoding(
                offset,
                "IAm max APDU length: expected application-tagged unsigned",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at max-apdu-length"));
        }
        let max_apdu_length_raw = primitives::decode_unsigned(&data[pos..end])?;
        let max_apdu_length = u32::try_from(max_apdu_length_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("IAm max APDU length {max_apdu_length_raw} exceeds u32"),
            )
        })?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::ENUMERATED
            || data[offset] & 0x07 > 5
        {
            return Err(Error::decoding(
                offset,
                "IAm segmentation: expected application-tagged enumerated",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at segmentation"));
        }
        let seg_raw = primitives::decode_unsigned(&data[pos..end])?;
        let seg_raw = u8::try_from(seg_raw)
            .map_err(|_| Error::decoding(pos, format!("IAm segmentation {seg_raw} exceeds u8")))?;
        let segmentation_supported = Segmentation::from_raw(seg_raw);
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::UNSIGNED
            || data[offset] & 0x07 > 5
        {
            return Err(Error::decoding(
                offset,
                "IAm vendor ID: expected application-tagged unsigned",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at vendor-id"));
        }
        let vendor_id_raw = primitives::decode_unsigned(&data[pos..end])?;
        let vendor_id = u16::try_from(vendor_id_raw).map_err(|_| {
            Error::decoding(pos, format!("IAm vendor ID {vendor_id_raw} exceeds u16"))
        })?;

        Ok(Self {
            object_identifier,
            max_apdu_length,
            segmentation_supported,
            vendor_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn who_is_all_round_trip() {
        let req = WhoIsRequest::all();
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(buf.is_empty());
        let decoded = WhoIsRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn who_is_range_round_trip() {
        let req = WhoIsRequest::range(1000, 2000);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(!buf.is_empty());
        let decoded = WhoIsRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn i_am_round_trip() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 999,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = IAmRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn i_am_wire_format() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 42,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        // First byte should be app tag 12, length 4 = 0xC4
        assert_eq!(buf[0], 0xC4);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_who_is_truncated() {
        // WhoIs with range: encode valid, then truncate to only first tag byte
        let req = WhoIsRequest::range(1000, 2000);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // Truncate to just the first tag + partial value (missing high-limit)
        // This should still decode as "all" because only one limit is present
        // Actually truncating at 1 byte should cause tag decode error
        assert!(WhoIsRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_who_is_invalid_tag() {
        // Non-empty but with non-matching context tags — decoder treats as unbounded
        let result = WhoIsRequest::decode(&[0x29, 0]).unwrap();
        assert_eq!(result.low_limit, None);
        assert_eq!(result.high_limit, None);
    }

    #[test]
    fn who_is_low_exceeds_high_is_error() {
        let req = WhoIsRequest::range(2000, 1000);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let err = WhoIsRequest::decode(&buf).unwrap_err();
        assert!(
            format!("{err:?}").contains("low_limit exceeds high_limit"),
            "expected low_limit > high_limit error, got: {err:?}"
        );
    }

    #[test]
    fn who_is_equal_limits_is_valid() {
        let req = WhoIsRequest::range(1500, 1500);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WhoIsRequest::decode(&buf).unwrap();
        assert_eq!(decoded.low_limit, Some(1500));
        assert_eq!(decoded.high_limit, Some(1500));
    }

    #[test]
    fn who_is_limits_must_fit_u32() {
        let encode_range = |low, high| {
            let mut buf = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut buf, 0, low);
            primitives::encode_ctx_unsigned(&mut buf, 1, high);
            buf
        };

        for (low, high, field, value) in [
            (4_294_967_297, 4_294_967_297, "low-limit", 4_294_967_297_u64),
            (1, 4_294_967_297, "high-limit", 4_294_967_297),
        ] {
            let encoded = encode_range(low, high);
            let error = WhoIsRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("WhoIs {field} {value}")),
                "unexpected error for {field} {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        for tag_number in [0, 1] {
            tags::encode_tag(&mut leading_zero, tag_number, tags::TagClass::Context, 5);
            leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        }
        let decoded = WhoIsRequest::decode(&leading_zero).unwrap();
        assert_eq!(decoded.low_limit, Some(u32::MAX));
        assert_eq!(decoded.high_limit, Some(u32::MAX));
    }

    #[test]
    fn i_am_values_must_fit_field_widths() {
        let object_identifier = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        let encode_request = |max_apdu_length, segmentation, vendor_id| {
            let mut buf = BytesMut::new();
            primitives::encode_app_object_id(&mut buf, &object_identifier);
            primitives::encode_app_unsigned(&mut buf, max_apdu_length);
            primitives::encode_app_enumerated(&mut buf, segmentation);
            primitives::encode_app_unsigned(&mut buf, vendor_id);
            buf
        };

        for (max_apdu_length, segmentation, vendor_id, field, value) in [
            (4_294_967_296, 0, 0, "max APDU length", 4_294_967_296_u64),
            (1, 256, 0, "segmentation", 256),
            (1, 0, 65_536, "vendor ID", 65_536),
        ] {
            let encoded = encode_request(max_apdu_length, segmentation, vendor_id);
            let error = IAmRequest::decode(&encoded).unwrap_err();
            assert!(
                error.to_string().contains(&format!("IAm {field} {value}")),
                "unexpected error for {field} {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        primitives::encode_app_object_id(&mut leading_zero, &object_identifier);
        for (tag_number, content) in [
            (tags::app_tag::UNSIGNED, &[0, 0xff, 0xff, 0xff, 0xff][..]),
            (tags::app_tag::ENUMERATED, &[0, 0xff][..]),
            (tags::app_tag::UNSIGNED, &[0, 0xff, 0xff][..]),
        ] {
            tags::encode_tag(
                &mut leading_zero,
                tag_number,
                tags::TagClass::Application,
                content.len() as u32,
            );
            leading_zero.extend_from_slice(content);
        }
        let decoded = IAmRequest::decode(&leading_zero).unwrap();
        assert_eq!(decoded.max_apdu_length, u32::MAX);
        assert_eq!(decoded.segmentation_supported.to_raw(), u8::MAX);
        assert_eq!(decoded.vendor_id, u16::MAX);

        let encode_with_tags = |object_tag, max_apdu_tag, segmentation_tag, vendor_tag| {
            let mut buf = BytesMut::new();
            for (tag_number, content) in [
                (object_tag, &object_identifier.encode()[..]),
                (max_apdu_tag, &1476_u16.to_be_bytes()[..]),
                (segmentation_tag, &[0][..]),
                (vendor_tag, &999_u16.to_be_bytes()[..]),
            ] {
                tags::encode_tag(
                    &mut buf,
                    tag_number,
                    tags::TagClass::Application,
                    content.len() as u32,
                );
                buf.extend_from_slice(content);
            }
            buf
        };
        for (object_tag, max_apdu_tag, segmentation_tag, vendor_tag, field) in [
            (
                tags::app_tag::UNSIGNED,
                tags::app_tag::UNSIGNED,
                tags::app_tag::ENUMERATED,
                tags::app_tag::UNSIGNED,
                "object identifier",
            ),
            (
                tags::app_tag::OBJECT_IDENTIFIER,
                tags::app_tag::ENUMERATED,
                tags::app_tag::ENUMERATED,
                tags::app_tag::UNSIGNED,
                "max APDU length",
            ),
            (
                tags::app_tag::OBJECT_IDENTIFIER,
                tags::app_tag::UNSIGNED,
                tags::app_tag::UNSIGNED,
                tags::app_tag::UNSIGNED,
                "segmentation",
            ),
            (
                tags::app_tag::OBJECT_IDENTIFIER,
                tags::app_tag::UNSIGNED,
                tags::app_tag::ENUMERATED,
                tags::app_tag::SIGNED,
                "vendor ID",
            ),
        ] {
            let encoded = encode_with_tags(object_tag, max_apdu_tag, segmentation_tag, vendor_tag);
            let error = IAmRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("IAm {field}: expected application-tagged")),
                "unexpected error for {field} tag: {error}"
            );
        }

        let valid = encode_request(1476, 0, 999);
        let mut reserved_lvt = BytesMut::from(&[0xc6, 4][..]);
        reserved_lvt.extend_from_slice(&valid[1..]);
        assert!(IAmRequest::decode(&reserved_lvt).is_err());
    }

    #[test]
    fn test_decode_i_am_empty_input() {
        assert!(IAmRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_i_am_truncated_1_byte() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 999,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(IAmRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_i_am_truncated_2_bytes() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 999,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(IAmRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_i_am_truncated_3_bytes() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 999,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(IAmRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_i_am_truncated_half() {
        let req = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 999,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(IAmRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_i_am_invalid_tag() {
        assert!(IAmRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
