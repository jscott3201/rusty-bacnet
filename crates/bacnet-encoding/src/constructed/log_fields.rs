//! The fields the three log record codecs share: the constructed timestamp,
//! the three-bit log status, the error pair, bit-string checks and the
//! any-value wrapper.
//!
//! Every record of Clause 21's BACnetLogRecord, BACnetEventLogRecord and
//! BACnetLogMultipleRecord opens with context 0 around an application Date
//! then an application Time, and each record kind reuses the same encodings
//! for its log-status, failure and any-value alternatives under its own tag
//! numbers. `record` names the record kind in error messages, and `what`
//! names the record kind and the field.

#[cfg(not(feature = "std"))]
use alloc::{string::ToString, vec::Vec};

use super::tagged::{contents, decode_ctx_constructed, expect_end};
use super::validate_tlv_sequence;
use crate::{primitives, tags};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::error::Error;
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
    what: &str,
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
    record: &str,
) -> Result<LogStatus, Error> {
    match contents {
        [5, bits] if bits & 0x1f == 0 => Ok(LogStatus::from_bacnet(&[*bits])),
        _ => Err(Error::decoding(
            offset,
            format!("{record} log-status must be a canonical three-bit BitString"),
        )),
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
        return Err(Error::decoding(
            offset,
            format!("integer-value needs 1 to 8 octets, got {}", contents.len()),
        ));
    }
    let fill = if contents[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let mut octets = [fill; 8];
    octets[8 - contents.len()..].copy_from_slice(contents);
    Ok(i64::from_be_bytes(octets))
}

/// Write the error pair as constructed context `tag` around an application
/// Enumerated class then code.
pub(super) fn encode_failure(buf: &mut BytesMut, tag: u8, error_class: u32, error_code: u32) {
    tags::encode_opening_tag(buf, tag);
    primitives::encode_app_enumerated(buf, error_class);
    primitives::encode_app_enumerated(buf, error_code);
    tags::encode_closing_tag(buf, tag);
}

/// The class and code in the body of a failure alternative.
pub(super) fn decode_failure(body: &[u8], offset: usize, what: &str) -> Result<(u32, u32), Error> {
    let (class, next) = application(body, 0, tags::app_tag::ENUMERATED, 0, offset, what)?;
    let (code, last) = application(body, next, tags::app_tag::ENUMERATED, 0, offset, what)?;
    expect_end(body, last, offset, what)?;
    Ok((
        primitives::decode_unsigned_u32(class)?,
        primitives::decode_unsigned_u32(code)?,
    ))
}

/// Refuse a bit string whose unused-bit count is above 7, or nonzero with no
/// data: the decoder would refuse it. `what` names the value in the error.
pub(super) fn check_bit_string(unused_bits: u8, data: &[u8], what: &str) -> Result<(), Error> {
    if unused_bits > 7 || (data.is_empty() && unused_bits != 0) {
        return Err(Error::OutOfRange(format!(
            "{what} has {unused_bits} unused bits over {} octets",
            data.len()
        )));
    }
    Ok(())
}

/// Write an any-value as constructed context `tag` around `bytes`, which
/// must be complete tagged values with every context tag balanced, so that
/// wrapping them cannot unbalance the record.
pub(super) fn encode_any_value(buf: &mut BytesMut, tag: u8, bytes: &[u8]) -> Result<(), Error> {
    validate_tlv_sequence(bytes, "any-value")
        .map_err(|error| Error::Encoding(error.to_string()))?;
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

/// The contents of the application `number` value at `pos`, with exactly
/// `length` octets unless `length` is zero.
fn application<'a>(
    data: &'a [u8],
    pos: usize,
    number: u8,
    length: u32,
    offset: usize,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, start) = tags::decode_tag(data, pos)?;
    if tag.class != tags::TagClass::Application
        || tag.number != number
        || (length != 0 && tag.length != length)
    {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application tag {number}"),
        ));
    }
    contents(data, start, tag.length)
}
