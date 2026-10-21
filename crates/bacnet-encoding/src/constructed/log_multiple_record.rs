//! Trend Log Multiple log records (Clause 12.30.19), framed as Clause 21
//! gives BACnetLogMultipleRecord.
//!
//! A record is two constructed fields. Context tag 0 wraps the timestamp, an
//! application Date then an application Time. Context tag 1 wraps the log
//! data, one of three alternatives: a primitive context 0 holding the
//! three-bit log status, a constructed context 1 holding one entry per
//! logged member, or a primitive context 2 holding the clock change as a
//! REAL. Inside the member list each entry has its own context tag, 0 to 8
//! in [`LogValue`] declaration order: the seven plain datatypes as
//! primitives, then failure as a constructed pair of application
//! Enumerated class and code, then any-value as a constructed wrapper
//! around the value's own tagged encoding.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::log_fields::{
    check_bit_string, decode_any_value, decode_failure, decode_integer, decode_log_status,
    decode_timestamp, encode_any_value, encode_ctx_integer, encode_failure, encode_log_status,
    encode_timestamp,
};
use super::tagged::{contents, decode_ctx_constructed, expect_end};
use crate::{primitives, tags};
use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogValue};
use bacnet_types::error::Error;
use bytes::BytesMut;

const RECORD: &str = "BACnetLogMultipleRecord";
const TIMESTAMP: &str = "BACnetLogMultipleRecord timestamp";
const LOG_DATA: &str = "BACnetLogMultipleRecord log-data";
const FAILURE_FIELD: &str = "BACnetLogMultipleRecord failure";

const BOOLEAN: u8 = 0;
const REAL: u8 = 1;
const ENUMERATED: u8 = 2;
const UNSIGNED: u8 = 3;
const INTEGER: u8 = 4;
const BIT_STRING: u8 = 5;
const NULL: u8 = 6;
const FAILURE: u8 = 7;
const ANY: u8 = 8;

/// Encode one Trend Log Multiple record.
///
/// Fails, leaving `buf` unchanged, for a bit string whose unused-bit count
/// is above 7 (or nonzero with no data) or an any-value whose bytes aren't
/// complete tagged values.
pub fn encode_log_multiple_record(
    record: &BACnetLogMultipleRecord,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let mut out = BytesMut::new();
    encode_timestamp(&mut out, &record.date, &record.time);
    tags::encode_opening_tag(&mut out, 1);
    match &record.log_data {
        LogData::LogStatus(status) => encode_log_status(&mut out, 0, *status),
        LogData::Values(values) => {
            tags::encode_opening_tag(&mut out, 1);
            for value in values {
                encode_value(value, &mut out)?;
            }
            tags::encode_closing_tag(&mut out, 1);
        }
        LogData::TimeChange(seconds) => primitives::encode_ctx_real(&mut out, 2, *seconds),
    }
    tags::encode_closing_tag(&mut out, 1);
    buf.extend_from_slice(&out);
    Ok(())
}

fn encode_value(value: &LogValue, buf: &mut BytesMut) -> Result<(), Error> {
    match value {
        LogValue::BooleanValue(value) => primitives::encode_ctx_boolean(buf, BOOLEAN, *value),
        LogValue::RealValue(value) => primitives::encode_ctx_real(buf, REAL, *value),
        LogValue::EnumValue(value) => primitives::encode_ctx_unsigned(buf, ENUMERATED, *value),
        LogValue::UnsignedValue(value) => primitives::encode_ctx_unsigned(buf, UNSIGNED, *value),
        LogValue::SignedValue(value) => encode_ctx_integer(buf, INTEGER, *value),
        LogValue::BitstringValue { unused_bits, data } => {
            check_bit_string(
                *unused_bits,
                data,
                "bitstring-value has {} unused bits over {} octets",
            )?;
            primitives::encode_ctx_bit_string(buf, BIT_STRING, *unused_bits, data);
        }
        LogValue::NullValue => tags::encode_tag(buf, NULL, tags::TagClass::Context, 0),
        LogValue::Failure {
            error_class,
            error_code,
        } => encode_failure(buf, FAILURE, *error_class, *error_code),
        LogValue::AnyValue(bytes) => encode_any_value(buf, ANY, bytes)?,
    }
    Ok(())
}

/// Decode one Trend Log Multiple record starting at `offset`, returning it
/// and the offset just past it.
pub fn decode_log_multiple_record(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetLogMultipleRecord, usize), Error> {
    let (date, time, data_start) = decode_timestamp(data, offset, TIMESTAMP)?;
    let (body, end) = decode_ctx_constructed(data, data_start, 1, LOG_DATA)?;
    let log_data = decode_log_data(body, data_start)?;
    Ok((
        BACnetLogMultipleRecord {
            date,
            time,
            log_data,
        },
        end,
    ))
}

fn decode_log_data(data: &[u8], offset: usize) -> Result<LogData, Error> {
    let (tag, start) = tags::decode_tag(data, 0)?;
    let (log_data, end) = if tag.is_context(0) {
        let (octets, end) = contents(data, start, tag.length)?;
        (
            LogData::LogStatus(decode_log_status(octets, offset, RECORD)?),
            end,
        )
    } else if tag.is_opening_tag(1) {
        let (list, end) = tags::extract_context_value(data, start, 1)?;
        (LogData::Values(decode_values(list, offset)?), end)
    } else if tag.is_context(2) {
        let (octets, end) = contents(data, start, tag.length)?;
        (LogData::TimeChange(primitives::decode_real(octets)?), end)
    } else {
        return Err(Error::decoding(
            offset,
            "log-data expected context [0], constructed [1], or context [2]",
        ));
    };
    expect_end(data, end, offset, LOG_DATA)?;
    Ok(log_data)
}

fn decode_values(data: &[u8], offset: usize) -> Result<Vec<LogValue>, Error> {
    let mut values = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (tag, start) = tags::decode_tag(data, pos)?;
        let (value, end) = if tag.is_opening_tag(FAILURE) {
            let (body, end) = tags::extract_context_value(data, start, FAILURE)?;
            let (error_class, error_code) = decode_failure(body, offset, FAILURE_FIELD)?;
            let value = LogValue::Failure {
                error_class,
                error_code,
            };
            (value, end)
        } else if tag.is_opening_tag(ANY) {
            let (body, end) = tags::extract_context_value(data, start, ANY)?;
            (LogValue::AnyValue(decode_any_value(body)?), end)
        } else if tag.class == tags::TagClass::Context && !tag.is_opening && !tag.is_closing {
            let (octets, end) = contents(data, start, tag.length)?;
            (decode_primitive(tag.number, octets, offset)?, end)
        } else {
            return Err(Error::decoding(offset, "log-data entry has an unknown tag"));
        };
        values.push(value);
        pos = end;
    }
    Ok(values)
}

fn decode_primitive(number: u8, contents: &[u8], offset: usize) -> Result<LogValue, Error> {
    Ok(match number {
        BOOLEAN => match contents {
            [0] => LogValue::BooleanValue(false),
            [1] => LogValue::BooleanValue(true),
            _ => return Err(Error::decoding(offset, "boolean-value must be one octet")),
        },
        REAL => LogValue::RealValue(primitives::decode_real(contents)?),
        ENUMERATED => LogValue::EnumValue(primitives::decode_unsigned(contents)?),
        UNSIGNED => LogValue::UnsignedValue(primitives::decode_unsigned(contents)?),
        INTEGER => LogValue::SignedValue(decode_integer(contents, offset)?),
        BIT_STRING => {
            let (unused_bits, data) = primitives::decode_bit_string(contents)?;
            LogValue::BitstringValue { unused_bits, data }
        }
        NULL if contents.is_empty() => LogValue::NullValue,
        _ => return Err(Error::decoding(offset, "log-data entry has an unknown tag")),
    })
}
