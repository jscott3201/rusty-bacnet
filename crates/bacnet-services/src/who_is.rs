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
            low_limit = Some(primitives::decode_unsigned(&data[pos..end])? as u32);
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
                high_limit = Some(primitives::decode_unsigned(&data[pos..end])? as u32);
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
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at object-identifier"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at max-apdu-length"));
        }
        let max_apdu_length = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at segmentation"));
        }
        let seg_raw = primitives::decode_unsigned(&data[pos..end])? as u8;
        let segmentation_supported = Segmentation::from_raw(seg_raw);
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "IAm truncated at vendor-id"));
        }
        let vendor_id = primitives::decode_unsigned(&data[pos..end])? as u16;

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
        let result = WhoIsRequest::decode(&[0xFF, 0xFF]).unwrap();
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
