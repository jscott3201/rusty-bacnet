//! Clause 21 codecs for an Accumulator's Scale and Prescale (Clause 12.61,
//! Table 12-79).
//!
//! - `BACnetScale` is a CHOICE of two primitive context tags: `[0]` a REAL
//!   for a float scale, `[1]` an INTEGER for a power-of-ten scale. A value is
//!   that one tag, with no frame around it.
//! - `BACnetPrescale` is a SEQUENCE of two required primitive context tags:
//!   `[0]` the multiplier and `[1]` the modulo divide, both Unsigned.

use bacnet_types::constructed::{BACnetPrescale, BACnetScale};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use super::tagged::{decode_ctx_primitive, decode_ctx_real, decode_ctx_unsigned, misplaced};
use crate::{primitives, tags};

const SCALE: &str = "BACnetScale";
const PRESCALE: &str = "BACnetPrescale";

/// Encode one `BACnetScale` CHOICE, appending to `buf`.
pub fn encode_scale(buf: &mut BytesMut, scale: &BACnetScale) {
    match *scale {
        BACnetScale::FloatScale(factor) => primitives::encode_ctx_real(buf, 0, factor),
        BACnetScale::IntegerScale(power) => primitives::encode_ctx_signed(buf, 1, power),
    }
}

/// Decode one `BACnetScale` CHOICE at `offset`.
///
/// Returns the scale and the offset just past it; a caller holding exactly
/// one value must check that offset reaches the end of its data. No tag at
/// all is [`DecodingKind::Missing`], and any tag but a primitive context
/// `[0]` or `[1]` the kind [`misplaced`] gives (InvalidTag, or Missing for
/// a closing tag that ends the enclosing frame). A REAL that isn't four
/// octets or an INTEGER of no octets or more than four is
/// [`DecodingKind::InvalidEncoding`], and truncated contents
/// [`Error::BufferTooShort`].
///
/// [`DecodingKind::Missing`]: bacnet_types::error::DecodingKind::Missing
/// [`DecodingKind::InvalidEncoding`]: bacnet_types::error::DecodingKind::InvalidEncoding
pub fn decode_scale(data: &[u8], offset: usize) -> Result<(BACnetScale, usize), Error> {
    let (tag, _) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) {
        let (factor, end) = decode_ctx_real(data, offset, 0, SCALE)?;
        return Ok((BACnetScale::FloatScale(factor), end));
    }
    if tag.is_context(1) {
        let (octets, end) = decode_ctx_primitive(data, offset, 1, SCALE)?;
        let power = primitives::decode_signed(octets).map_err(|_| {
            DecodeError::new(
                DecodingKind::InvalidEncoding,
                offset,
                "integer-scale [1] needs 1 to 4 octets",
            )
            .within(SCALE)
        })?;
        return Ok((BACnetScale::IntegerScale(power), end));
    }
    Err(misplaced(
        data,
        &tag,
        None,
        offset,
        "expected float-scale [0] or integer-scale [1]",
    )
    .within(SCALE)
    .into())
}

/// Encode one `BACnetPrescale` SEQUENCE, appending to `buf`.
pub fn encode_prescale(buf: &mut BytesMut, prescale: &BACnetPrescale) {
    primitives::encode_ctx_unsigned(buf, 0, u64::from(prescale.multiplier));
    primitives::encode_ctx_unsigned(buf, 1, u64::from(prescale.modulo_divide));
}

/// Decode one `BACnetPrescale` SEQUENCE at `offset`; returns it and the
/// offset past its `[1]` member. A member left out (the data ends, or `[1]`
/// comes first) is [`DecodingKind::Missing`], another tag where a member is
/// due [`DecodingKind::InvalidTag`], an Unsigned of no octets
/// [`DecodingKind::InvalidEncoding`], and a value past unsigned32, the width
/// the object keeps, [`DecodingKind::OutOfRange`].
///
/// [`DecodingKind::Missing`]: bacnet_types::error::DecodingKind::Missing
/// [`DecodingKind::InvalidTag`]: bacnet_types::error::DecodingKind::InvalidTag
/// [`DecodingKind::InvalidEncoding`]: bacnet_types::error::DecodingKind::InvalidEncoding
/// [`DecodingKind::OutOfRange`]: bacnet_types::error::DecodingKind::OutOfRange
pub fn decode_prescale(data: &[u8], offset: usize) -> Result<(BACnetPrescale, usize), Error> {
    let (multiplier, pos) = decode_ctx_unsigned::<u32>(data, offset, 0, PRESCALE)?;
    let (modulo_divide, end) = decode_ctx_unsigned::<u32>(data, pos, 1, PRESCALE)?;
    Ok((
        BACnetPrescale {
            multiplier,
            modulo_divide,
        },
        end,
    ))
}
