use super::log_fields::{decode_log_status, encode_log_status};
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

/// Encode one adjacent `BACnetAuditLogRecordResult` sequence.
pub fn encode_audit_log_record_result(
    result: &BACnetAuditLogRecordResult,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    primitives::encode_ctx_unsigned(buf, 0, result.sequence_number);
    tags::encode_opening_tag(buf, 1);
    encode_audit_log_record(&result.record, buf)?;
    tags::encode_closing_tag(buf, 1);
    Ok(())
}

/// Encode one bare `BACnetAuditLogRecord` field sequence.
pub fn encode_audit_log_record(
    record: &BACnetAuditLogRecord,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    validate_date_time(&record.timestamp.0, &record.timestamp.1)?;

    tags::encode_opening_tag(buf, 0);
    primitives::encode_app_date(buf, &record.timestamp.0);
    primitives::encode_app_time(buf, &record.timestamp.1);
    tags::encode_closing_tag(buf, 0);

    tags::encode_opening_tag(buf, 1);
    match &record.datum {
        BACnetAuditLogDatum::LogStatus(status) => encode_log_status(buf, 0, *status),
        BACnetAuditLogDatum::AuditNotification(notification) => {
            tags::encode_opening_tag(buf, 1);
            encode_audit_notification(notification, buf)?;
            tags::encode_closing_tag(buf, 1);
        }
        BACnetAuditLogDatum::TimeChange(change) => {
            primitives::encode_ctx_real(buf, 2, *change);
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
    let (choice, _) = tags::decode_tag(data, 0)?;
    if choice.is_context(0) {
        const WHAT: &str = "BACnetAuditLogRecord log-status";
        let (contents, end) = decode_ctx_primitive(data, 0, 0, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        Ok(BACnetAuditLogDatum::LogStatus(decode_log_status(
            contents,
            0,
            "BACnetAuditLogRecord",
        )?))
    } else if choice.is_opening_tag(1) {
        const WHAT: &str = "AuditLogQuery-ACK AuditNotification choice";
        let (notification_body, end) = decode_ctx_constructed(data, 0, 1, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        let (notification, notification_end) = decode_audit_notification_at(notification_body, 0)?;
        expect_end(
            notification_body,
            notification_end,
            notification_end,
            "AuditNotification choice notification",
        )?;
        Ok(BACnetAuditLogDatum::AuditNotification(notification))
    } else if choice.is_context(2) {
        const WHAT: &str = "BACnetAuditLogRecord time-change";
        let (contents, end) = decode_ctx_primitive(data, 0, 2, WHAT)?;
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
        return Err(Error::encode("BACnetAuditLogRecord timestamp contains an invalid Date or Time component"));
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
