//! Trend Log log records (Clause 12.25.14), framed as Clause 21 gives
//! BACnetLogRecord.
//!
//! A record is two constructed fields and an optional third. Context tag 0
//! wraps the timestamp, an application Date then an application Time.
//! Context tag 1 wraps the log datum, whose alternatives take context tags 0
//! to 10 in [`LogDatum`] declaration order: the three-bit log status, the
//! seven plain datatypes and the clock change as primitives, failure as a
//! constructed pair of application Enumerated class and code, and any-value
//! as a constructed wrapper around the value's own tagged encoding. A
//! primitive context 2 holding the four-bit Status_Flags may follow.

use super::log_fields::{
    check_bit_string, decode_any_value, decode_failure, decode_integer, decode_log_status,
    decode_timestamp, encode_any_value, encode_ctx_integer, encode_failure, encode_log_status,
    encode_timestamp,
};
use super::tagged::{contents, decode_ctx_constructed, expect_end};
use crate::{primitives, tags};
use bacnet_types::bitstring::status_flags_from_bacnet;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::error::Error;
use bacnet_types::primitives::StatusFlags;
use bytes::BytesMut;

const RECORD: &str = "BACnetLogRecord";
const TIMESTAMP: &str = "BACnetLogRecord timestamp";
const DATUM: &str = "BACnetLogRecord log-datum";
const FAILURE_FIELD: &str = "BACnetLogRecord failure";

const LOG_STATUS: u8 = 0;
const BOOLEAN: u8 = 1;
const REAL: u8 = 2;
const ENUMERATED: u8 = 3;
const UNSIGNED: u8 = 4;
const INTEGER: u8 = 5;
const BIT_STRING: u8 = 6;
const NULL: u8 = 7;
const FAILURE: u8 = 8;
const TIME_CHANGE: u8 = 9;
const ANY: u8 = 10;

/// The record field after the datum.
const STATUS_FLAGS: u8 = 2;

/// Encode one Trend Log record.
///
/// Fails, leaving `buf` unchanged, for a bit string whose unused-bit count
/// is above 7 (or nonzero with no data) or an any-value whose bytes aren't
/// complete tagged values.
pub fn encode_log_record(record: &BACnetLogRecord, buf: &mut BytesMut) -> Result<(), Error> {
    let mut out = BytesMut::new();
    encode_timestamp(&mut out, &record.date, &record.time);
    tags::encode_opening_tag(&mut out, 1);
    match &record.log_datum {
        LogDatum::LogStatus(status) => encode_log_status(&mut out, LOG_STATUS, *status),
        LogDatum::BooleanValue(value) => primitives::encode_ctx_boolean(&mut out, BOOLEAN, *value),
        LogDatum::RealValue(value) => primitives::encode_ctx_real(&mut out, REAL, *value),
        // An ENUMERATED's contents are encoded as an Unsigned's are.
        LogDatum::EnumValue(value) => primitives::encode_ctx_unsigned(&mut out, ENUMERATED, *value),
        LogDatum::UnsignedValue(value) => {
            primitives::encode_ctx_unsigned(&mut out, UNSIGNED, *value)
        }
        LogDatum::SignedValue(value) => encode_ctx_integer(&mut out, INTEGER, *value),
        LogDatum::BitstringValue { unused_bits, data } => {
            check_bit_string(
                *unused_bits,
                data,
                "bitstring-value has {} unused bits over {} octets",
            )?;
            primitives::encode_ctx_bit_string(&mut out, BIT_STRING, *unused_bits, data);
        }
        LogDatum::NullValue => tags::encode_tag(&mut out, NULL, tags::TagClass::Context, 0),
        LogDatum::Failure {
            error_class,
            error_code,
        } => encode_failure(&mut out, FAILURE, *error_class, *error_code),
        LogDatum::TimeChange(seconds) => {
            primitives::encode_ctx_real(&mut out, TIME_CHANGE, *seconds)
        }
        LogDatum::AnyValue(bytes) => encode_any_value(&mut out, ANY, bytes)?,
    }
    tags::encode_closing_tag(&mut out, 1);
    if let Some(flags) = record.status_flags {
        // StatusFlags keeps in-alarm as its highest of four bits, so shifting
        // puts it in the top bit of the octet, where the wire wants bit 0.
        let octet = (flags & StatusFlags::all()).bits() << 4;
        primitives::encode_ctx_bit_string(&mut out, STATUS_FLAGS, 4, &[octet]);
    }
    buf.extend_from_slice(&out);
    Ok(())
}

/// Decode one Trend Log record starting at `offset`, returning it and the
/// offset just past it.
///
/// Records sit back to back in a ReadRange item list, so the optional status
/// flags are read only when the very next tag is the primitive context 2;
/// anything else is left for the caller as the start of what follows.
pub fn decode_log_record(data: &[u8], offset: usize) -> Result<(BACnetLogRecord, usize), Error> {
    let (date, time, datum_start) = decode_timestamp(data, offset, TIMESTAMP)?;
    let (body, mut end) = decode_ctx_constructed(data, datum_start, 1, DATUM)?;
    let log_datum = decode_datum(body, datum_start)?;
    let mut status_flags = None;
    if end < data.len() {
        let (tag, start) = tags::decode_tag(data, end)?;
        if tag.is_context(STATUS_FLAGS) {
            let (octets, next) = contents(data, start, tag.length)?;
            status_flags = match octets {
                [4, bits] if bits & 0x0f == 0 => Some(status_flags_from_bacnet(&[*bits])),
                _ => {
                    return Err(Error::decoding(
                        end,
                        "BACnetLogRecord status-flags must be a four-bit BitString",
                    ))
                }
            };
            end = next;
        }
    }
    Ok((
        BACnetLogRecord {
            date,
            time,
            log_datum,
            status_flags,
        },
        end,
    ))
}

fn decode_datum(data: &[u8], offset: usize) -> Result<LogDatum, Error> {
    let (tag, start) = tags::decode_tag(data, 0)?;
    let (datum, end) = if tag.is_opening_tag(FAILURE) {
        let (body, end) = tags::extract_context_value(data, start, FAILURE)?;
        let (error_class, error_code) = decode_failure(body, offset, FAILURE_FIELD)?;
        let datum = LogDatum::Failure {
            error_class,
            error_code,
        };
        (datum, end)
    } else if tag.is_opening_tag(ANY) {
        let (body, end) = tags::extract_context_value(data, start, ANY)?;
        (LogDatum::AnyValue(decode_any_value(body)?), end)
    } else if tag.class == tags::TagClass::Context && !tag.is_opening && !tag.is_closing {
        let (octets, end) = contents(data, start, tag.length)?;
        (decode_primitive(tag.number, octets, offset)?, end)
    } else {
        return Err(Error::decoding(offset, "log-datum has an unknown tag"));
    };
    expect_end(data, end, offset, DATUM)?;
    Ok(datum)
}

fn decode_primitive(number: u8, contents: &[u8], offset: usize) -> Result<LogDatum, Error> {
    Ok(match number {
        LOG_STATUS => LogDatum::LogStatus(decode_log_status(contents, offset, RECORD)?),
        BOOLEAN => match contents {
            [0] => LogDatum::BooleanValue(false),
            [1] => LogDatum::BooleanValue(true),
            _ => return Err(Error::decoding(offset, "boolean-value must be one octet")),
        },
        REAL => LogDatum::RealValue(primitives::decode_real(contents)?),
        ENUMERATED => LogDatum::EnumValue(primitives::decode_unsigned(contents)?),
        UNSIGNED => LogDatum::UnsignedValue(primitives::decode_unsigned(contents)?),
        INTEGER => LogDatum::SignedValue(decode_integer(contents, offset)?),
        BIT_STRING => {
            let (unused_bits, data) = primitives::decode_bit_string(contents)?;
            LogDatum::BitstringValue { unused_bits, data }
        }
        NULL if contents.is_empty() => LogDatum::NullValue,
        TIME_CHANGE => LogDatum::TimeChange(primitives::decode_real(contents)?),
        _ => return Err(Error::decoding(offset, "log-datum has an unknown tag")),
    })
}
