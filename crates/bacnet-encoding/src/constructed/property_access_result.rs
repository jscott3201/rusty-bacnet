//! `BACnetPropertyAccessResult` (ASHRAE 135-2020 Clause 21), the element of a
//! Global Group's Present_Value array (Clause 12.50.7).
//!
//! An element opens with the members of a `BACnetDeviceObjectPropertyReference`
//! under the same context tags: the object `[0]`, the property `[1]`, then the
//! optional array index `[2]` and device `[3]`. The result follows as one of
//! two constructed alternatives: the value read inside an opening/closing
//! context tag `[4]` pair, or the error inside a `[5]` pair, carried as an
//! application-tagged ENUMERATED class and then code. An array of these
//! elements concatenates them with no frame.

use bacnet_types::constructed::{AccessResult, BACnetPropertyAccessResult};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use super::tagged::{decode_app_enumerated, decode_framed_value, expect_closing};
use super::{decode_dopr_body, encode_dopr_body};
use crate::{primitives, tags};

const WHAT: &str = "BACnetPropertyAccessResult";

/// Encode one bare `BACnetPropertyAccessResult`, appending to `buf`.
///
/// Fails only when the value itself can't be encoded (see
/// [`primitives::encode_property_value`]), and then leaves `buf` as it was;
/// the reference and an error result always encode.
pub fn encode_property_access_result(
    buf: &mut BytesMut,
    result: &BACnetPropertyAccessResult,
) -> Result<(), Error> {
    let start = buf.len();
    encode_dopr_body(buf, &result.reference);
    match &result.access_result {
        AccessResult::Value(value) => {
            tags::encode_opening_tag(buf, 4);
            if let Err(error) = primitives::encode_property_value(buf, value) {
                buf.truncate(start);
                return Err(error);
            }
            tags::encode_closing_tag(buf, 4);
        }
        AccessResult::Error { class, code } => {
            tags::encode_opening_tag(buf, 5);
            primitives::encode_app_enumerated(buf, u32::from(class.to_raw()));
            primitives::encode_app_enumerated(buf, u32::from(code.to_raw()));
            tags::encode_closing_tag(buf, 5);
        }
    }
    Ok(())
}

/// Decode one bare `BACnetPropertyAccessResult` at `offset`; returns it and
/// the offset past its closing `[4]` or `[5]` tag, so an array is walked by
/// calling this at each element's start.
///
/// The value inside `[4]` decodes as one application value when it holds
/// one element, and as a [`PropertyValue::List`] otherwise (an array read
/// whole, empty included). A context-tagged element in it decodes to
/// [`PropertyValue::ApplicationData`], so the value re-encodes to the same
/// octets.
///
/// [`PropertyValue::List`]: bacnet_types::primitives::PropertyValue::List
/// [`PropertyValue::ApplicationData`]: bacnet_types::primitives::PropertyValue::ApplicationData
pub fn decode_property_access_result(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetPropertyAccessResult, usize), Error> {
    let (reference, offset) = decode_dopr_body(data, offset, WHAT)?;
    let (tag, content) = tags::decode_tag(data, offset)?;
    let (access_result, end) = if tag.is_opening_tag(4) {
        let (value, end) = decode_framed_value(data, content, 4, WHAT)?;
        (AccessResult::Value(value), end)
    } else if tag.is_opening_tag(5) {
        let (class, next) = decode_app_enumerated::<u32>(data, content, WHAT)?;
        let (code, next) = decode_app_enumerated::<u32>(data, next, WHAT)?;
        let class = u16::try_from(class).map_err(|_| {
            DecodeError::new(DecodingKind::OutOfRange, content, "error class exceeds u16")
                .within(WHAT)
        })?;
        let code = u16::try_from(code).map_err(|_| {
            DecodeError::new(DecodingKind::OutOfRange, content, "error code exceeds u16")
                .within(WHAT)
        })?;
        let end = expect_closing(data, next, 5, WHAT)?;
        let error = AccessResult::Error {
            class: ErrorClass::from_raw(class),
            code: ErrorCode::from_raw(code),
        };
        (error, end)
    } else {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "expected property-value [4] or property-access-error [5]",
        )
        .within(WHAT)
        .into());
    };
    Ok((
        BACnetPropertyAccessResult {
            reference,
            access_result,
        },
        end,
    ))
}
