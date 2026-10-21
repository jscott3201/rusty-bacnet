//! Clause 21 codec for `BACnetShedLevel`, the datatype of the Load Control
//! object's Requested_Shed_Level, Expected_Shed_Level and Actual_Shed_Level
//! (Clause 12.28).
//!
//! The CHOICE has three alternatives, each a primitive context tag: percent
//! `[0]` and level `[1]` carry an Unsigned, amount `[2]` a REAL. A value is
//! that one tag, with no frame around it.

use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use super::tagged::contents;
use crate::{primitives, tags};

const WHAT: &str = "BACnetShedLevel";

/// Encode one `BACnetShedLevel` CHOICE, appending to `buf`.
pub fn encode_shed_level(buf: &mut BytesMut, value: &BACnetShedLevel) {
    match *value {
        BACnetShedLevel::Percent(percent) => primitives::encode_ctx_unsigned(buf, 0, percent),
        BACnetShedLevel::Level(level) => primitives::encode_ctx_unsigned(buf, 1, level),
        BACnetShedLevel::Amount(amount) => primitives::encode_ctx_real(buf, 2, amount),
    }
}

/// Decode one `BACnetShedLevel` CHOICE at `offset`.
///
/// Returns the value and the offset just past it; a caller holding exactly
/// one value must check that offset reaches the end of its data. Any tag but
/// a primitive context `[0]`, `[1]` or `[2]`, an Unsigned of no octets or more
/// than eight, a REAL that isn't four octets, or truncated contents fails with
/// [`Error::Decoding`] or [`Error::BufferTooShort`]. The decoder checks
/// structure only: what range each alternative may take is up to the object.
pub fn decode_shed_level(data: &[u8], offset: usize) -> Result<(BACnetShedLevel, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    let alternative = (0..=2u8).find(|&number| tag.is_context(number));
    let Some(alternative) = alternative else {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "expected percent [0], level [1] or amount [2]",
        )
        .within(WHAT)
        .into());
    };
    let (contents, end) = contents(data, pos, tag.length)?;
    let refused = |label: &'static str| {
        DecodeError::new(DecodingKind::InvalidEncoding, pos, label).within(WHAT)
    };
    let unsigned =
        |label: &'static str| primitives::decode_unsigned(contents).map_err(|_| refused(label));
    let value = match alternative {
        0 => BACnetShedLevel::Percent(unsigned("percent [0] needs 1 to 8 octets")?),
        1 => BACnetShedLevel::Level(unsigned("level [1] needs 1 to 8 octets")?),
        _ => BACnetShedLevel::Amount(
            primitives::decode_real(contents).map_err(|_| refused("amount [2] needs 4 octets"))?,
        ),
    };
    Ok((value, end))
}
