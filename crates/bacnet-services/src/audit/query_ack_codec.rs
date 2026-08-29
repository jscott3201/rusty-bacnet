use super::{
    decode_canonical_unsigned, notification_codec, AuditLogQueryAck, BACnetAuditLogDatum,
    BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use crate::common::{decode_context, decode_context_bool, MAX_DECODED_ITEMS};
use bacnet_encoding::{primitives, tags};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

const AUDIT_LOG_TAG: u8 = 0;
const RECORDS_TAG: u8 = 1;
const NO_MORE_ITEMS_TAG: u8 = 2;

pub(super) fn encode(ack: &AuditLogQueryAck, buf: &mut BytesMut) -> Result<(), Error> {
    if ack.records.len() > MAX_DECODED_ITEMS {
        return Err(Error::OutOfRange(format!(
            "AuditLogQuery-ACK record count {} exceeds {MAX_DECODED_ITEMS}",
            ack.records.len()
        )));
    }

    // Build the complete payload separately so a later invalid record cannot
    // leave a partial complex-ACK in the caller's APDU buffer.
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_object_id(&mut encoded, AUDIT_LOG_TAG, &ack.audit_log);
    tags::encode_opening_tag(&mut encoded, RECORDS_TAG);
    for result in &ack.records {
        encode_result(result, &mut encoded)?;
    }
    tags::encode_closing_tag(&mut encoded, RECORDS_TAG);
    primitives::encode_ctx_boolean(&mut encoded, NO_MORE_ITEMS_TAG, ack.no_more_items);

    buf.extend_from_slice(&encoded);
    Ok(())
}

fn encode_result(result: &BACnetAuditLogRecordResult, buf: &mut BytesMut) -> Result<(), Error> {
    primitives::encode_ctx_unsigned(buf, 0, result.sequence_number);
    tags::encode_opening_tag(buf, 1);
    encode_record(&result.record, buf)?;
    tags::encode_closing_tag(buf, 1);
    Ok(())
}

fn encode_record(record: &BACnetAuditLogRecord, buf: &mut BytesMut) -> Result<(), Error> {
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
            notification_codec::encode_notification(notification, buf)?;
            tags::encode_closing_tag(buf, 1);
        }
        BACnetAuditLogDatum::TimeChange(change) => {
            primitives::encode_ctx_real(buf, 2, *change);
        }
    }
    tags::encode_closing_tag(buf, 1);
    Ok(())
}

pub(super) fn decode(data: &[u8]) -> Result<AuditLogQueryAck, Error> {
    let (audit_log_contents, mut offset) =
        decode_context(data, 0, AUDIT_LOG_TAG, "AuditLogQuery-ACK audit-log")?;
    let audit_log = ObjectIdentifier::decode(audit_log_contents)?;

    let (records_body, records_end) =
        decode_constructed_body(data, offset, RECORDS_TAG, "record list")?;
    let mut records = Vec::new();
    let mut record_offset = 0;
    while record_offset < records_body.len() {
        if records.len() >= MAX_DECODED_ITEMS {
            return Err(Error::decoding(
                offset + record_offset,
                format!("AuditLogQuery-ACK record count exceeds {MAX_DECODED_ITEMS}"),
            ));
        }
        let (result, next) = decode_result(records_body, record_offset)?;
        if next <= record_offset {
            return Err(Error::decoding(
                offset + record_offset,
                "AuditLogQuery-ACK record decoder made no progress",
            ));
        }
        records.push(result);
        record_offset = next;
    }
    offset = records_end;

    let (no_more_items, end) = decode_context_bool(
        data,
        offset,
        NO_MORE_ITEMS_TAG,
        "AuditLogQuery-ACK no-more-items",
    )?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            "AuditLogQuery-ACK has trailing service data",
        ));
    }

    Ok(AuditLogQueryAck {
        audit_log,
        records,
        no_more_items,
    })
}

fn decode_result(data: &[u8], offset: usize) -> Result<(BACnetAuditLogRecordResult, usize), Error> {
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
    let record = decode_record(record_body)?;
    Ok((
        BACnetAuditLogRecordResult {
            sequence_number,
            record,
        },
        next,
    ))
}

fn decode_record(data: &[u8]) -> Result<BACnetAuditLogRecord, Error> {
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
        let (notification, notification_end) =
            notification_codec::decode_notification(notification_body, 0)?;
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
