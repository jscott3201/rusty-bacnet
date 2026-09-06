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
        BACnetAuditLogDatum::LogStatus(status) => {
            if status & !0b111 != 0 {
                return Err(Error::OutOfRange(format!(
                    "BACnetAuditLogRecord log-status {status:#010b} exceeds three bits"
                )));
            }
            primitives::encode_ctx_bit_string(buf, 0, 5, &[*status << 5]);
        }
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
    let (sequence_contents, record_start) =
        decode_context(data, offset, 0, "AuditLogQuery-ACK record sequence-number")?;
    if sequence_contents.is_empty() || sequence_contents.len() > 8 {
        return Err(Error::decoding(
            offset,
            "AuditLogQuery-ACK sequence-number must contain one to eight octets",
        ));
    }
    let sequence_number = decode_canonical_unsigned(
        sequence_contents,
        offset,
        "AuditLogQuery-ACK sequence-number",
    )?;

    let (record_body, next) = decode_constructed_body(data, record_start, 1, "record value")?;
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
    let (timestamp_body, datum_start) = decode_constructed_body(data, 0, 0, "record timestamp")?;
    let timestamp = decode_date_time(timestamp_body)?;

    let (datum_body, end) = decode_constructed_body(data, datum_start, 1, "record datum")?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            "BACnetAuditLogRecord has trailing fields",
        ));
    }
    let datum = decode_datum(datum_body)?;

    Ok(BACnetAuditLogRecord { timestamp, datum })
}

fn decode_date_time(data: &[u8]) -> Result<(Date, Time), Error> {
    let (date_tag, date_start) = tags::decode_tag(data, 0)?;
    if date_tag.class != tags::TagClass::Application
        || date_tag.number != tags::app_tag::DATE
        || date_tag.is_opening
        || date_tag.is_closing
        || date_tag.length != 4
    {
        return Err(Error::decoding(
            0,
            "BACnetAuditLogRecord timestamp expected four-octet application Date",
        ));
    }
    let date_end = date_start
        .checked_add(4)
        .ok_or_else(|| Error::decoding(date_start, "timestamp Date length overflow"))?;
    if date_end > data.len() {
        return Err(Error::decoding(date_start, "timestamp Date is truncated"));
    }
    let date = Date::decode(&data[date_start..date_end])?;

    let (time_tag, time_start) = tags::decode_tag(data, date_end)?;
    if time_tag.class != tags::TagClass::Application
        || time_tag.number != tags::app_tag::TIME
        || time_tag.is_opening
        || time_tag.is_closing
        || time_tag.length != 4
    {
        return Err(Error::decoding(
            date_end,
            "BACnetAuditLogRecord timestamp expected four-octet application Time",
        ));
    }
    let time_end = time_start
        .checked_add(4)
        .ok_or_else(|| Error::decoding(time_start, "timestamp Time length overflow"))?;
    if time_end > data.len() {
        return Err(Error::decoding(time_start, "timestamp Time is truncated"));
    }
    if time_end != data.len() {
        return Err(Error::decoding(
            time_end,
            "BACnetAuditLogRecord timestamp has trailing fields",
        ));
    }
    let time = Time::decode(&data[time_start..time_end])?;
    validate_date_time(&date, &time).map_err(|error| {
        Error::decoding(
            0,
            format!("BACnetAuditLogRecord timestamp is malformed: {error}"),
        )
    })?;
    Ok((date, time))
}

fn decode_datum(data: &[u8]) -> Result<BACnetAuditLogDatum, Error> {
    let (choice, _) = tags::decode_tag(data, 0)?;
    if choice.is_context(0) {
        let (contents, end) = decode_context(data, 0, 0, "BACnetAuditLogRecord log-status")?;
        if end != data.len() {
            return Err(Error::decoding(
                end,
                "log-status choice has trailing fields",
            ));
        }
        if contents.len() != 2 || contents[0] != 5 || contents[1] & 0x1f != 0 {
            return Err(Error::decoding(
                0,
                "BACnetAuditLogRecord log-status must be a canonical three-bit BitString",
            ));
        }
        Ok(BACnetAuditLogDatum::LogStatus(contents[1] >> 5))
    } else if choice.is_opening_tag(1) {
        let (notification_body, end) =
            decode_constructed_body(data, 0, 1, "AuditNotification choice")?;
        if end != data.len() {
            return Err(Error::decoding(
                end,
                "AuditNotification choice has trailing fields",
            ));
        }
        let (notification, notification_end) = decode_audit_notification_at(notification_body, 0)?;
        if notification_end != notification_body.len() {
            return Err(Error::decoding(
                notification_end,
                "AuditNotification choice has trailing notification fields",
            ));
        }
        Ok(BACnetAuditLogDatum::AuditNotification(notification))
    } else if choice.is_context(2) {
        let (contents, end) = decode_context(data, 0, 2, "BACnetAuditLogRecord time-change")?;
        if end != data.len() {
            return Err(Error::decoding(
                end,
                "time-change choice has trailing fields",
            ));
        }
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

fn decode_constructed_body<'a>(
    data: &'a [u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (opening, body_start) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("AuditLogQuery-ACK {field} expected opening tag [{tag_number}]"),
        ));
    }
    tags::extract_context_value(data, body_start, tag_number)
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
        return Err(Error::Encoding(
            "BACnetAuditLogRecord timestamp contains an invalid Date or Time component".into(),
        ));
    }
    Ok(())
}

fn decode_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, contents_start) = tags::decode_tag(data, offset)?;
    if !tag.is_context(expected_tag) {
        return Err(Error::decoding(
            offset,
            format!("{field} expected context tag [{expected_tag}]"),
        ));
    }
    let end = contents_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(contents_start, format!("{field} length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((&data[contents_start..end], end))
}

fn decode_canonical_unsigned(data: &[u8], offset: usize, field: &str) -> Result<u64, Error> {
    if data.is_empty() || data.len() > 8 {
        return Err(Error::decoding(
            offset,
            format!("{field} must contain one to eight octets"),
        ));
    }
    if data.len() > 1 && data.first() == Some(&0) {
        return Err(Error::decoding(
            offset,
            format!("{field} must use the shortest Unsigned/Enumerated encoding"),
        ));
    }
    primitives::decode_unsigned(data)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            datum: BACnetAuditLogDatum::LogStatus(0b010),
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
