//! Audit Log records and AuditLogQuery results, framed as Clause 21 gives
//! BACnetAuditLogRecord and BACnetAuditLogRecordResult.
//!
//! A record is two constructed fields: context 0 around the application
//! Date and Time, and context 1 around the datum, a primitive context 0
//! holding the three-bit log status, a constructed context 1 holding an
//! audit notification, or a primitive context 2 holding the clock change
//! as a REAL. An alternative under a higher tag, primitive or constructed,
//! is kept as [`BACnetAuditLogDatum::Opaque`] and written back unchanged
//! (#2483), with the bounds the other log records' CHOICEs keep.

use super::log_fields::{
    IN_AUDIT_RESULT, check_record, decode_log_status, decode_opaque, encode_log_status,
    encode_opaque,
};
use super::tagged::{
    decode_app_fixed, decode_ctx_canonical_unsigned, decode_ctx_constructed, decode_ctx_primitive,
    expect_end,
};
use super::{decode_audit_notification_at, encode_audit_notification};
use crate::{primitives, tags};
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

const RECORD: &str = "BACnetAuditLogRecord";
const DATUM: &str = "BACnetAuditLogRecord log-datum";

const LOG_STATUS: u8 = 0;
const NOTIFICATION: u8 = 1;
const TIME_CHANGE: u8 = 2;

/// Whether the log-datum CHOICE gives `tag` no alternative: above
/// time-change, the last it assigns.
fn unassigned(tag: u8) -> bool {
    tag > TIME_CHANGE
}

/// Encode one adjacent `BACnetAuditLogRecordResult` sequence, leaving `buf`
/// unchanged when the record doesn't encode.
pub fn encode_audit_log_record_result(
    result: &BACnetAuditLogRecordResult,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let start = buf.len();
    primitives::encode_ctx_unsigned(buf, 0, result.sequence_number);
    tags::encode_opening_tag(buf, 1);
    encode_audit_log_record(&result.record, buf).inspect_err(|_| buf.truncate(start))?;
    tags::encode_closing_tag(buf, 1);
    Ok(())
}

/// Encode one bare `BACnetAuditLogRecord` field sequence.
///
/// Fails, leaving `buf` unchanged, for a timestamp that isn't a date and
/// time, a notification that would not encode, an opaque datum under a tag
/// the CHOICE assigns or whose body isn't complete tagged values, or a
/// record nested too deep for an AuditLogQuery-ACK to carry (#2483), which
/// [`is_record_too_deep`](super::is_record_too_deep) tells apart.
pub fn encode_audit_log_record(
    record: &BACnetAuditLogRecord,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    validate_date_time(&record.timestamp.0, &record.timestamp.1)?;
    // Straight into `buf`, cut back on failure: no staging buffer (#2233).
    let start = buf.len();
    write_audit_log_record(record, buf)
        .and_then(|()| check_record(&buf[start..], IN_AUDIT_RESULT, RECORD))
        .inspect_err(|_| buf.truncate(start))
}

fn write_audit_log_record(record: &BACnetAuditLogRecord, buf: &mut BytesMut) -> Result<(), Error> {
    tags::encode_opening_tag(buf, 0);
    primitives::encode_app_date(buf, &record.timestamp.0);
    primitives::encode_app_time(buf, &record.timestamp.1);
    tags::encode_closing_tag(buf, 0);

    tags::encode_opening_tag(buf, 1);
    match &record.datum {
        BACnetAuditLogDatum::LogStatus(status) => encode_log_status(buf, LOG_STATUS, *status),
        BACnetAuditLogDatum::AuditNotification(notification) => {
            tags::encode_opening_tag(buf, NOTIFICATION);
            encode_audit_notification(notification, buf)?;
            tags::encode_closing_tag(buf, NOTIFICATION);
        }
        BACnetAuditLogDatum::TimeChange(change) => {
            primitives::encode_ctx_real(buf, TIME_CHANGE, *change);
        }
        BACnetAuditLogDatum::Opaque(opaque) => {
            encode_opaque(buf, opaque, unassigned, DATUM)?;
        }
    }
    tags::encode_closing_tag(buf, 1);
    Ok(())
}

/// Decode one adjacent result sequence starting at `offset`.
pub fn decode_audit_log_record_result_at(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetAuditLogRecordResult, usize), Error> {
    let (sequence_number, record_start) = decode_ctx_canonical_unsigned::<u64>(
        data,
        offset,
        0,
        "AuditLogQuery-ACK record sequence-number",
    )?;
    let (record_body, next) =
        decode_ctx_constructed(data, record_start, 1, "AuditLogQuery-ACK record value")?;
    let record = decode_audit_log_record(record_body)?;
    Ok((
        BACnetAuditLogRecordResult {
            sequence_number,
            record,
        },
        next,
    ))
}

/// Decode a complete bare `BACnetAuditLogRecord` field sequence.
pub fn decode_audit_log_record(data: &[u8]) -> Result<BACnetAuditLogRecord, Error> {
    let (record, end) = decode_audit_log_record_at(data, 0)?;
    expect_end(data, end, end, "BACnetAuditLogRecord")?;
    Ok(record)
}

/// Decode one bare `BACnetAuditLogRecord` starting at `offset`, returning it
/// and the offset just past it, so the records of a ReadRange item list
/// decode one after another.
pub fn decode_audit_log_record_at(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetAuditLogRecord, usize), Error> {
    let (timestamp_body, datum_start) =
        decode_ctx_constructed(data, offset, 0, "AuditLogQuery-ACK record timestamp")?;
    let timestamp = decode_date_time(timestamp_body)?;
    let (datum_body, end) =
        decode_ctx_constructed(data, datum_start, 1, "AuditLogQuery-ACK record datum")?;
    let datum = decode_datum(datum_body)?;
    Ok((BACnetAuditLogRecord { timestamp, datum }, end))
}

fn decode_date_time(data: &[u8]) -> Result<(Date, Time), Error> {
    const WHAT: &str = "BACnetAuditLogRecord timestamp";
    let (date, date_end) = decode_app_fixed(data, 0, tags::app_tag::DATE, 4, WHAT)?;
    let (time, time_end) = decode_app_fixed(data, date_end, tags::app_tag::TIME, 4, WHAT)?;
    expect_end(data, time_end, time_end, WHAT)?;
    let date = Date::decode(date)?;
    let time = Time::decode(time)?;
    validate_date_time(&date, &time).map_err(|_| {
        Error::decoding(
            0,
            "BACnetAuditLogRecord timestamp contains an invalid Date or Time component",
        )
    })?;
    Ok((date, time))
}

fn decode_datum(data: &[u8]) -> Result<BACnetAuditLogDatum, Error> {
    let (choice, start) = tags::decode_tag(data, 0)?;
    if let Some((opaque, end)) = decode_opaque(data, &choice, start, unassigned)? {
        expect_end(data, end, end, DATUM)?;
        Ok(BACnetAuditLogDatum::Opaque(opaque))
    } else if choice.is_context(LOG_STATUS) {
        const WHAT: &str = "BACnetAuditLogRecord log-status";
        let (contents, end) = decode_ctx_primitive(data, 0, LOG_STATUS, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        Ok(BACnetAuditLogDatum::LogStatus(decode_log_status(
            contents, 0, RECORD,
        )?))
    } else if choice.is_opening_tag(NOTIFICATION) {
        const WHAT: &str = "AuditLogQuery-ACK AuditNotification choice";
        let (notification_body, end) = decode_ctx_constructed(data, 0, NOTIFICATION, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        let (notification, notification_end) = decode_audit_notification_at(notification_body, 0)?;
        expect_end(
            notification_body,
            notification_end,
            notification_end,
            "AuditNotification choice notification",
        )?;
        Ok(BACnetAuditLogDatum::AuditNotification(notification))
    } else if choice.is_context(TIME_CHANGE) {
        const WHAT: &str = "BACnetAuditLogRecord time-change";
        let (contents, end) = decode_ctx_primitive(data, 0, TIME_CHANGE, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        Ok(BACnetAuditLogDatum::TimeChange(primitives::decode_real(
            contents,
        )?))
    } else {
        Err(Error::decoding(
            0,
            "BACnetAuditLogRecord datum expected context [0], constructed [1], or context [2]",
        ))
    }
}

fn validate_date_time(date: &Date, time: &Time) -> Result<(), Error> {
    let date_valid = (date.month == Date::UNSPECIFIED || (1..=14).contains(&date.month))
        && (date.day == Date::UNSPECIFIED || (1..=34).contains(&date.day))
        && (date.day_of_week == Date::UNSPECIFIED || (1..=7).contains(&date.day_of_week));
    let time_valid = (time.hour == Time::UNSPECIFIED || time.hour <= 23)
        && (time.minute == Time::UNSPECIFIED || time.minute <= 59)
        && (time.second == Time::UNSPECIFIED || time.second <= 59)
        && (time.hundredths == Time::UNSPECIFIED || time.hundredths <= 99);
    if !date_valid || !time_valid {
        return Err(Error::encode(
            "BACnetAuditLogRecord timestamp contains an invalid Date or Time component",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::bitstring::LogStatus;

    fn audit_record() -> BACnetAuditLogRecord {
        BACnetAuditLogRecord {
            timestamp: (
                Date {
                    year: 124,
                    month: 2,
                    day: 29,
                    day_of_week: 4,
                },
                Time {
                    hour: 12,
                    minute: 34,
                    second: 56,
                    hundredths: 78,
                },
            ),
            datum: BACnetAuditLogDatum::LogStatus(LogStatus::BUFFER_PURGED),
        }
    }

    /// The log-status choice puts log-disabled, bit 0, in the top bit of the
    /// octet (Clause 20.2.10), so it never reads back as log-interrupted.
    #[test]
    fn audit_record_log_status_has_bit0_first_wire_bytes() {
        for (status, octet) in [
            (LogStatus::LOG_DISABLED, 0x80),
            (LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED, 0xC0),
            (LogStatus::LOG_INTERRUPTED, 0x20),
        ] {
            let record = BACnetAuditLogRecord {
                datum: BACnetAuditLogDatum::LogStatus(status),
                ..audit_record()
            };
            let mut encoded = BytesMut::new();
            encode_audit_log_record(&record, &mut encoded).unwrap();
            // Timestamp [0], then [1] around log-status [0] = 05 <octet>.
            assert_eq!(&encoded[12..], &[0x1E, 0x0A, 0x05, octet, 0x1F], "{status}");
            assert_eq!(decode_audit_log_record(&encoded).unwrap(), record);
        }
    }

    #[test]
    fn audit_record_shared_codec_round_trips_and_rejects_trailing_data() {
        let expected = audit_record();
        let mut encoded = BytesMut::new();
        encode_audit_log_record(&expected, &mut encoded).unwrap();
        assert_eq!(decode_audit_log_record(&encoded).unwrap(), expected);

        encoded.extend_from_slice(&[0]);
        assert!(decode_audit_log_record(&encoded).is_err());
    }

    #[test]
    fn consecutive_audit_records_decode_by_returned_offset() {
        let first = audit_record();
        let second = BACnetAuditLogRecord {
            datum: BACnetAuditLogDatum::TimeChange(-1.5),
            ..audit_record()
        };
        let mut encoded = BytesMut::new();
        encode_audit_log_record(&first, &mut encoded).unwrap();
        let split = encoded.len();
        encode_audit_log_record(&second, &mut encoded).unwrap();
        assert_eq!(
            decode_audit_log_record_at(&encoded, 0).unwrap(),
            (first, split)
        );
        assert_eq!(
            decode_audit_log_record_at(&encoded, split).unwrap(),
            (second, encoded.len())
        );
        assert!(decode_audit_log_record_at(&encoded[..encoded.len() - 1], split).is_err());
    }

    #[test]
    fn audit_record_result_preserves_u64_sequence_identity() {
        let expected = BACnetAuditLogRecordResult {
            sequence_number: u64::MAX,
            record: audit_record(),
        };
        let mut encoded = BytesMut::new();
        encode_audit_log_record_result(&expected, &mut encoded).unwrap();
        let (decoded, end) = decode_audit_log_record_result_at(&encoded, 0).unwrap();
        assert_eq!(end, encoded.len());
        assert_eq!(decoded, expected);
    }
}
