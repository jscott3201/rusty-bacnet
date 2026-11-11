//! The fields the three log record codecs share: the constructed timestamp,
//! the three-bit log status, the error pair, bit-string checks, the
//! any-value wrapper and an alternative under a tag the CHOICE doesn't
//! assign.
//!
//! Every record of Clause 21's BACnetLogRecord, BACnetEventLogRecord and
//! BACnetLogMultipleRecord opens with context 0 around an application Date
//! then an application Time, and each record kind reuses the same encodings
//! for its log-status, failure and any-value alternatives under its own tag
//! numbers. `record` names the record kind in error messages, and `what`
//! names the record kind and the field.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::tagged::{contents, decode_ctx_constructed, expect_end};
use super::validate_tlv_sequence;
use crate::{primitives, tags};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::OpaqueChoice;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{DecodeError, DecodingKind, EncodeError, Error, RangeError};
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

/// Write the timestamp field: context 0 around an application Date and Time.
pub(super) fn encode_timestamp(buf: &mut BytesMut, date: &Date, time: &Time) {
    tags::encode_opening_tag(buf, 0);
    primitives::encode_app_date(buf, date);
    primitives::encode_app_time(buf, time);
    tags::encode_closing_tag(buf, 0);
}

/// Read the timestamp field opening at `offset`, returning it and the offset
/// of the field after it.
pub(super) fn decode_timestamp(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(Date, Time, usize), Error> {
    let (body, end) = decode_ctx_constructed(data, offset, 0, what)?;
    let (date, next) = application(body, 0, tags::app_tag::DATE, 4, offset, what)?;
    let (time, last) = application(body, next, tags::app_tag::TIME, 4, offset, what)?;
    expect_end(body, last, offset, what)?;
    Ok((Date::decode(date)?, Time::decode(time)?, end))
}

/// Write a log status as the primitive context `tag`: a BIT STRING of three
/// bits, log-disabled in the top bit of the octet (Clause 20.2.10).
pub(super) fn encode_log_status(buf: &mut BytesMut, tag: u8, status: LogStatus) {
    primitives::encode_ctx_bit_string(buf, tag, 5, &[status.to_bacnet()]);
}

/// The log status in a primitive's `contents`: exactly a three-bit BIT
/// STRING with its padding clear.
pub(super) fn decode_log_status(
    contents: &[u8],
    offset: usize,
    record: &'static str,
) -> Result<LogStatus, Error> {
    match contents {
        [5, bits] if bits & 0x1f == 0 => Ok(LogStatus::from_bacnet(&[*bits])),
        _ => Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "log-status must be a canonical three-bit BitString",
        )
        .within(record)
        .into()),
    }
}

/// Write an INTEGER of up to 64 bits as the primitive context `tag`, in the
/// fewest octets two's complement allows.
pub(super) fn encode_ctx_integer(buf: &mut BytesMut, tag: u8, value: i64) {
    let octets = value.to_be_bytes();
    // Drop leading octets that only repeat the sign of the next one.
    let start = (0..7)
        .find(|&i| {
            let redundant = (octets[i] == 0x00 && octets[i + 1] & 0x80 == 0)
                || (octets[i] == 0xFF && octets[i + 1] & 0x80 != 0);
            !redundant
        })
        .unwrap_or(7);
    tags::encode_tag(buf, tag, tags::TagClass::Context, (8 - start) as u32);
    buf.extend_from_slice(&octets[start..]);
}

/// An INTEGER of one to eight octets. A logging device may hold integers to
/// 32 bits but need not, so a record read from a peer can carry more.
pub(super) fn decode_integer(contents: &[u8], offset: usize) -> Result<i64, Error> {
    if contents.is_empty() || contents.len() > 8 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "integer-value needs 1 to 8 octets, got {}",
        )
        .with_value(contents.len() as u64)
        .into());
    }
    let fill = if contents[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let mut octets = [fill; 8];
    octets[8 - contents.len()..].copy_from_slice(contents);
    Ok(i64::from_be_bytes(octets))
}

/// Write the error pair as constructed context `tag` around an application
/// Enumerated class then code.
pub(super) fn encode_failure(
    buf: &mut BytesMut,
    tag: u8,
    error_class: ErrorClass,
    error_code: ErrorCode,
) {
    tags::encode_opening_tag(buf, tag);
    primitives::encode_app_enumerated(buf, error_class.to_raw().into());
    primitives::encode_app_enumerated(buf, error_code.to_raw().into());
    tags::encode_closing_tag(buf, tag);
}

/// The class and code in the body of a failure alternative. Clause 21 holds
/// both to 16 bits, so a wider one is refused, as the Error-PDU decoder
/// refuses it (#1814).
pub(super) fn decode_failure(
    body: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(ErrorClass, ErrorCode), Error> {
    let (class, next) = application(body, 0, tags::app_tag::ENUMERATED, 0, offset, what)?;
    let (code, last) = application(body, next, tags::app_tag::ENUMERATED, 0, offset, what)?;
    expect_end(body, last, offset, what)?;
    let sixteen = |raw: &[u8], message: &'static str| -> Result<u16, Error> {
        let value = primitives::decode_unsigned_u32(raw)?;
        u16::try_from(value).map_err(|_| {
            DecodeError::new(DecodingKind::OutOfRange, offset, message)
                .within(what)
                .with_value(value.into())
                .into()
        })
    };
    Ok((
        ErrorClass::from_raw(sixteen(class, "error class {} exceeds u16")?),
        ErrorCode::from_raw(sixteen(code, "error code {} exceeds u16")?),
    ))
}

/// Refuse a bit string whose unused-bit count is above 7, or nonzero with no
/// data: the decoder would refuse it. `label` names the value and holds a
/// placeholder for the unused-bit count, then one for the octet count.
pub(super) fn check_bit_string(
    unused_bits: u8,
    data: &[u8],
    label: &'static str,
) -> Result<(), Error> {
    if unused_bits > 7 || (data.is_empty() && unused_bits != 0) {
        return Err(RangeError::new(label)
            .with_values(unused_bits.into(), data.len() as u64)
            .into());
    }
    Ok(())
}

/// Write an any-value as constructed context `tag` around `bytes`, which
/// must be complete tagged values with every context tag balanced, so that
/// wrapping them cannot unbalance the record.
pub(super) fn encode_any_value(buf: &mut BytesMut, tag: u8, bytes: &[u8]) -> Result<(), Error> {
    validate_tlv_sequence(bytes, "any-value").map_err(Error::into_encode_refusal)?;
    tags::encode_opening_tag(buf, tag);
    buf.extend_from_slice(bytes);
    tags::encode_closing_tag(buf, tag);
    Ok(())
}

/// The contents of an any-value whose body is `body`, held to the rule its
/// encoder applies.
pub(super) fn decode_any_value(body: &[u8]) -> Result<Vec<u8>, Error> {
    validate_tlv_sequence(body, "any-value")?;
    Ok(body.to_vec())
}

/// The alternative whose tag `tag` starts at `start` in `data`, kept as
/// received when the tag is above `last`, the highest its CHOICE assigns:
/// `None` for any other tag, which the caller decodes or refuses. A
/// constructed one's body must be complete tagged values with every context
/// tag balanced, the rule an any-value's body follows, so a malformed or
/// truncated one still fails, as does nesting past the tag decoder's depth
/// limit.
pub(super) fn decode_opaque(
    data: &[u8],
    tag: &tags::Tag,
    start: usize,
    last: u8,
) -> Result<Option<(OpaqueChoice, usize)>, Error> {
    if tag.class != tags::TagClass::Context || tag.is_closing || tag.number <= last {
        return Ok(None);
    }
    let (data, end) = if tag.is_opening {
        let (body, end) = tags::extract_context_value(data, start, tag.number)?;
        validate_tlv_sequence(body, "unknown choice")?;
        (body, end)
    } else {
        contents(data, start, tag.length)?
    };
    let opaque = OpaqueChoice {
        tag: tag.number,
        constructed: tag.is_opening,
        data: data.to_vec(),
    };
    Ok(Some((opaque, end)))
}

/// Write an alternative [`decode_opaque`] kept back as it came: under its
/// own tag, primitive or between an opening and closing tag pair. Refuses a
/// tag up to `last`, which its CHOICE assigns, or 255, a primitive longer
/// than the tag decoder takes, and a constructed body that isn't complete
/// tagged values. `what` names the field in errors.
pub(super) fn encode_opaque(
    buf: &mut BytesMut,
    opaque: &OpaqueChoice,
    last: u8,
    what: &'static str,
) -> Result<(), Error> {
    let OpaqueChoice {
        tag,
        constructed,
        ref data,
    } = *opaque;
    if tag <= last || tag == u8::MAX {
        return Err(
            EncodeError::new("tag [{}] cannot carry an opaque alternative")
                .within(what)
                .with_value(tag.into())
                .into(),
        );
    }
    if constructed {
        validate_tlv_sequence(data, what).map_err(Error::into_encode_refusal)?;
        tags::encode_opening_tag(buf, tag);
        buf.extend_from_slice(data);
        tags::encode_closing_tag(buf, tag);
    } else {
        let length = u32::try_from(data.len())
            .ok()
            .filter(|&length| length <= tags::MAX_TAG_LENGTH)
            .ok_or_else(|| {
                Error::from(
                    EncodeError::new("opaque alternative over {} octets")
                        .within(what)
                        .with_value(tags::MAX_TAG_LENGTH),
                )
            })?;
        tags::encode_tag(buf, tag, tags::TagClass::Context, length);
        buf.extend_from_slice(data);
    }
    Ok(())
}

/// What a record encoder's refusal of a record nested too deep says, which
/// [`is_record_too_deep`] knows it by.
const TOO_DEEP: &str = "nests {} tags deep, past what a ReadRange-ACK carries";

/// Refuse a whole written record that a reader couldn't take back.
///
/// Each field was checked as it was written; what only the whole record
/// shows is how deep it nests, an any-value's or an opaque alternative's body
/// under the record's own levels. A ReadRange-ACK carries records inside its
/// item-data frame, and the tag decoder counts its nesting limit from that
/// frame, so a record may nest one level less deep than the limit. A stored
/// record past that would break every page of the log, and a persisted
/// snapshot holding it would not load.
///
/// The walk only follows the tags: no allocation, no recursion, and it
/// stops at the first opening tag past the limit.
pub(super) fn check_record(record: &[u8], what: &'static str) -> Result<(), Error> {
    let (mut depth, mut pos) = (0usize, 0usize);
    while pos < record.len() {
        let (tag, next) = tags::decode_tag(record, pos).map_err(Error::into_encode_refusal)?;
        pos = next;
        if tag.is_opening {
            depth += 1;
            if depth >= tags::MAX_CONTEXT_NESTING_DEPTH {
                return Err(EncodeError::new(TOO_DEEP)
                    .within(what)
                    .with_value(depth as u32)
                    .into());
            }
        } else if tag.is_closing {
            depth = depth.saturating_sub(1);
        } else if !(tag.class == tags::TagClass::Application
            && tag.number == tags::app_tag::BOOLEAN)
        {
            // A primitive's contents, which its field's own check read.
            pos = pos.saturating_add(tag.length as usize);
        }
    }
    Ok(())
}

/// Whether `error` is a log record encoder's refusal of a record nested
/// deeper than a ReadRange-ACK's item data carries (#2474): a record every
/// field of which is sound, so a reader may drop it and keep the rest, as
/// the log snapshot loader does with one an older build saved.
pub fn is_record_too_deep(error: &Error) -> bool {
    matches!(error, Error::Encode(refusal) if refusal.label == TOO_DEEP)
}

/// The contents of the application `number` value at `pos`, with exactly
/// `length` octets unless `length` is zero.
fn application<'a>(
    data: &'a [u8],
    pos: usize,
    number: u8,
    length: u32,
    offset: usize,
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, start) = tags::decode_tag(data, pos)?;
    if tag.class != tags::TagClass::Application
        || tag.number != number
        || (length != 0 && tag.length != length)
    {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "expected application tag {}",
        )
        .within(what)
        .with_value(number.into())
        .into());
    }
    contents(data, start, tag.length)
}
