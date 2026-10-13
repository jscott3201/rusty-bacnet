//! Who-Is and I-Am services per ASHRAE 135-2020 Clause 16.10.

use bacnet_encoding::constructed::tagged::{
    decode_app_enumerated, decode_app_object_id, decode_app_unsigned, decode_ctx_unsigned,
    decode_optional_ctx, expect_end,
};
use bacnet_encoding::primitives;
use bacnet_types::enums::Segmentation;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// DeviceInstanceRange
// ---------------------------------------------------------------------------

/// The devices a Who-Is or Who-Has asks to answer: those whose Device object
/// instance lies from [`low`](Self::low) to [`high`](Self::high), both
/// included (Clauses 16.10.1.1.1-2 and 16.9.1.1.1-2).
///
/// The clauses send both limits or neither, keep the low one at or below
/// the high one, and give each the instance range 0 to
/// [`ObjectIdentifier::MAX_INSTANCE`]. A request holds an
/// `Option<DeviceInstanceRange>`, so it can't carry one limit alone, and
/// [`Self::new`] refuses a low limit above the high one, a range no device
/// lies in, and a limit past the highest instance. The decoders refuse the
/// first two forms on the wire as well (#1447, #1483), but take a limit past
/// the highest instance, since some devices send one to mean every device;
/// [`Self::contains`] reads such a range as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceInstanceRange {
    low: u32,
    high: u32,
}

impl DeviceInstanceRange {
    /// The instances from `low` to `high`, both included. Fails with
    /// [`Error::OutOfRange`] when `low` is above `high`, or `high` is above
    /// [`ObjectIdentifier::MAX_INSTANCE`].
    pub fn new(low: u32, high: u32) -> Result<Self, Error> {
        let range = Self::decoded(low, high)?;
        if high > ObjectIdentifier::MAX_INSTANCE {
            return Err(Error::OutOfRange(format!(
                "device instance range high limit {high} is above the highest instance, {}",
                ObjectIdentifier::MAX_INSTANCE
            )));
        }
        Ok(range)
    }

    /// The range as a decoder reads it: `low` no greater than `high`, with
    /// either past the highest instance taken as written.
    fn decoded(low: u32, high: u32) -> Result<Self, Error> {
        if low > high {
            return Err(Error::OutOfRange(format!(
                "device instance range low limit {low} is above its high limit {high}, \
                 so it would match no device"
            )));
        }
        Ok(Self { low, high })
    }

    /// The one instance `instance`, as when a Who-Is looks for one device;
    /// fails as [`Self::new`] does for an instance past the highest.
    pub fn single(instance: u32) -> Result<Self, Error> {
        Self::new(instance, instance)
    }

    /// The one instance of `device`, which always lies in the instance range.
    pub fn device(device: ObjectIdentifier) -> Self {
        let instance = device.instance_number();
        Self {
            low: instance,
            high: instance,
        }
    }

    /// The range two separately held limits give: none for neither, a range
    /// for both, as the Python bindings take them. One limit alone fails with
    /// [`Error::OutOfRange`] naming it, rather than becoming a request for
    /// every device, and so does any range [`Self::new`] refuses.
    pub fn from_limits(low: Option<u32>, high: Option<u32>) -> Result<Option<Self>, Error> {
        match (low, high) {
            (None, None) => Ok(None),
            (Some(low), Some(high)) => Self::new(low, high).map(Some),
            (Some(low), None) => Err(Error::OutOfRange(format!(
                "a device instance range needs both limits or neither: \
                 low limit {low} was given without a high limit"
            ))),
            (None, Some(high)) => Err(Error::OutOfRange(format!(
                "a device instance range needs both limits or neither: \
                 high limit {high} was given without a low limit"
            ))),
        }
    }

    /// The lowest instance in the range.
    pub const fn low(self) -> u32 {
        self.low
    }

    /// The highest instance in the range.
    pub const fn high(self) -> u32 {
        self.high
    }

    /// Whether `instance` lies in the range.
    pub const fn contains(self, instance: u32) -> bool {
        self.low <= instance && instance <= self.high
    }

    /// Write the range as the `[0]` low and `[1]` high limits.
    pub(crate) fn encode(self, buf: &mut BytesMut) {
        primitives::encode_ctx_unsigned(buf, 0, self.low as u64);
        primitives::encode_ctx_unsigned(buf, 1, self.high as u64);
    }
}

/// What a Who-Is or Who-Has calls itself and its limits in errors. The
/// limits' labels are written out whole, so a request that decodes builds
/// none (#1645).
pub(crate) struct LimitLabels {
    /// The service, which names the pairing errors.
    service: &'static str,
    /// The `[0]` low limit.
    low: &'static str,
    /// The `[1]` high limit.
    high: &'static str,
}

impl LimitLabels {
    /// A Who-Is's.
    pub(crate) const WHO_IS: Self = Self {
        service: "WhoIs",
        low: "WhoIs low limit",
        high: "WhoIs high limit",
    };
    /// A Who-Has's.
    pub(crate) const WHO_HAS: Self = Self {
        service: "WhoHas",
        low: "WhoHas low limit",
        high: "WhoHas high limit",
    };
}

/// A Who-Is or Who-Has request's optional `[0]` low and `[1]` high limits,
/// as read from the start of its service-request octets. The decoders read
/// the rest of the request before [`Self::range`] checks the pairing.
pub(crate) struct WireLimits {
    low: Option<u32>,
    high: Option<u32>,
    /// Where a `[1]` high limit after a lone low limit was due.
    after_low: usize,
    /// The offset after the limits.
    pub(crate) end: usize,
    /// What errors call the service and its limits.
    labels: &'static LimitLabels,
}

impl WireLimits {
    /// Read the limits, which `labels` names in errors.
    pub(crate) fn decode(data: &[u8], labels: &'static LimitLabels) -> Result<Self, Error> {
        let (low, after_low) =
            decode_optional_ctx(data, 0, 0, labels.low, decode_ctx_unsigned::<u32>)?;
        let (high, end) =
            decode_optional_ctx(data, after_low, 1, labels.high, decode_ctx_unsigned::<u32>)?;
        Ok(Self {
            low,
            high,
            after_low,
            end,
            labels,
        })
    }

    /// The range the limits give. One limit without the other is malformed
    /// (#1447, #1483): a receiver drops it rather than reading it as a
    /// request for every device, which would make every device answer one
    /// that probably meant a range. So is a low limit above the high one.
    pub(crate) fn range(&self) -> Result<Option<DeviceInstanceRange>, Error> {
        let service = self.labels.service;
        match (self.low, self.high) {
            (None, None) => Ok(None),
            (Some(low), Some(high)) => {
                DeviceInstanceRange::decoded(low, high)
                    .map(Some)
                    .map_err(|_| {
                        Error::out_of_range(0, format!("{service} low limit exceeds high limit"))
                    })
            }
            (Some(_), None) => Err(Error::missing(
                self.after_low,
                format!("{service} low limit needs the high limit [1] with it"),
            )),
            (None, Some(_)) => Err(Error::missing(
                0,
                format!("{service} high limit needs the low limit [0] before it"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// WhoIsRequest
// ---------------------------------------------------------------------------

/// Who-Is-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoIsRequest {
    /// The devices that should answer; `None` asks every device.
    pub range: Option<DeviceInstanceRange>,
}

impl WhoIsRequest {
    /// Create an unbounded WhoIs (all devices).
    pub fn all() -> Self {
        Self { range: None }
    }

    /// Encode the request into `buf`: both limits for a range, nothing for an
    /// unbounded request.
    pub fn encode(&self, buf: &mut BytesMut) {
        if let Some(range) = self.range {
            range.encode(buf);
        }
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on any octet that isn't a `[0]` or `[1]` limit in its place, so a limit under another
    /// tag or anything after the limits refuses the request rather than reading as no limits,
    /// on one limit without the other (#1447), and on a low limit above the high one.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let limits = WireLimits::decode(data, &LimitLabels::WHO_IS)?;
        expect_end(data, limits.end, limits.end, "WhoIs")?;
        Ok(Self {
            range: limits.range()?,
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
    /// Device object of the announcing device.
    pub object_identifier: ObjectIdentifier,
    /// Largest APDU, in octets, the device can accept.
    pub max_apdu_length: u32,
    /// Segmentation abilities the device supports.
    pub segmentation_supported: Segmentation,
    /// Vendor identifier (Unsigned16) of the device manufacturer.
    pub vendor_id: u16,
}

impl IAmRequest {
    /// Encode the request parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.object_identifier);
        primitives::encode_app_unsigned(buf, self.max_apdu_length as u64);
        primitives::encode_app_enumerated(buf, self.segmentation_supported.to_raw() as u32);
        primitives::encode_app_unsigned(buf, self.vendor_id as u64);
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input
    /// and on octets after the vendor identifier.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (object_identifier, offset) = decode_app_object_id(data, 0, "IAm object identifier")?;
        let (max_apdu_length, offset) =
            decode_app_unsigned::<u32>(data, offset, "IAm max APDU length")?;
        let (segmentation, offset) = decode_app_enumerated::<u8>(data, offset, "IAm segmentation")?;
        let segmentation_supported = Segmentation::from_raw(segmentation);
        let (vendor_id, end) = decode_app_unsigned::<u16>(data, offset, "IAm vendor ID")?;
        expect_end(data, end, end, "IAm")?;

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
    use bacnet_encoding::tags;
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
        let req = WhoIsRequest {
            range: Some(DeviceInstanceRange::new(1000, 2000).unwrap()),
        };
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
        // A range cut anywhere fails: inside the low limit, or after it,
        // which leaves one limit alone.
        let req = WhoIsRequest {
            range: Some(DeviceInstanceRange::new(1000, 2000).unwrap()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        for cut in 1..buf.len() {
            assert!(WhoIsRequest::decode(&buf[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn who_is_with_one_limit_is_refused() {
        // [0] low limit 1 alone, then [1] high limit 10 alone (#1447).
        for data in [&[0x09, 0x01][..], &[0x19, 0x0A]] {
            let error = WhoIsRequest::decode(data).unwrap_err();
            assert!(matches!(error, Error::Decoding { .. }), "{error:?}");
        }
    }

    #[test]
    fn test_decode_who_is_invalid_tag() {
        // A tag that is neither limit is left over, not read as no limits.
        let error = WhoIsRequest::decode(&[0x29, 0]).unwrap_err();
        assert!(matches!(error, Error::Decoding { .. }), "{error:?}");
    }

    #[test]
    fn who_is_low_exceeds_high_is_error() {
        // Low limit 2000, high limit 1000: no range can be built this way,
        // so the octets are written out.
        let data = [0x0A, 0x07, 0xD0, 0x1A, 0x03, 0xE8];
        let err = WhoIsRequest::decode(&data).unwrap_err();
        assert!(
            err.to_string()
                .contains("WhoIs low limit exceeds high limit"),
            "expected a low limit above the high limit to fail, got: {err:?}"
        );
    }

    #[test]
    fn who_is_equal_limits_is_valid() {
        let req = WhoIsRequest {
            range: Some(DeviceInstanceRange::single(1500).unwrap()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert_eq!(buf[..], [0x0A, 0x05, 0xDC, 0x1A, 0x05, 0xDC]);
        let decoded = WhoIsRequest::decode(&buf).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn who_is_limits_must_fit_u32() {
        let encode_range = |low, high| {
            let mut buf = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut buf, 0, low);
            primitives::encode_ctx_unsigned(&mut buf, 1, high);
            buf
        };

        // The shared readers' wording names the member, its tag and the value.
        for (low, high, field, value) in [
            (
                4_294_967_297,
                4_294_967_297,
                "low limit: [0]",
                4_294_967_297_u64,
            ),
            (1, 4_294_967_297, "high limit: [1]", 4_294_967_297),
        ] {
            let encoded = encode_range(low, high);
            let error = WhoIsRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("WhoIs {field} value {value} exceeds u32")),
                "unexpected error for {field} {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        for tag_number in [0, 1] {
            tags::encode_tag(&mut leading_zero, tag_number, tags::TagClass::Context, 5);
            leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        }
        let decoded = WhoIsRequest::decode(&leading_zero).unwrap();
        // Past the highest instance, but taken as written.
        let range = decoded.range.unwrap();
        assert_eq!((range.low(), range.high()), (u32::MAX, u32::MAX));
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

        // The shared application readers name the member, its type and the
        // width it must fit.
        for (max_apdu_length, segmentation, vendor_id, refusal) in [
            (4_294_967_296, 0, 0, "max APDU length: Unsigned exceeds u32"),
            (1, 256, 0, "segmentation: ENUMERATED exceeds u8"),
            (1, 0, 65_536, "vendor ID: Unsigned exceeds u16"),
        ] {
            let encoded = encode_request(max_apdu_length, segmentation, vendor_id);
            let error = IAmRequest::decode(&encoded).unwrap_err();
            assert!(
                error.to_string().contains(&format!("IAm {refusal}")),
                "unexpected error for {refusal}: {error}"
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

#[cfg(test)]
#[path = "device_range_tests.rs"]
mod device_range_tests;
