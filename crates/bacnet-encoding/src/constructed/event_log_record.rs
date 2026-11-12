//! Event Log log records (Clause 12.27.13), framed as Clause 21 gives
//! BACnetEventLogRecord.
//!
//! A record is two constructed fields. Context tag 0 wraps the timestamp, an
//! application Date then an application Time. Context tag 1 wraps the log
//! datum, one of three alternatives: a primitive context 0 holding the
//! three-bit log status, a constructed context 1 holding a notification's
//! request parameters, or a primitive context 2 holding the clock change as
//! a REAL. An alternative under a higher tag, primitive or constructed, is
//! kept as [`EventLogDatum::Opaque`] and written back unchanged.

use super::log_fields::{
    IN_ITEM_DATA, check_record, decode_log_status, decode_opaque, decode_timestamp,
    encode_log_status, encode_opaque, encode_timestamp,
};
use super::tagged::{contents, decode_ctx_constructed, expect_end};
use super::{decode_event_notification_tolerant, encode_event_notification};
use crate::{primitives, tags};
use bacnet_types::constructed::{BACnetEventLogRecord, EventLogDatum};
use bacnet_types::error::Error;
use bytes::BytesMut;

const RECORD: &str = "BACnetEventLogRecord";
const TIMESTAMP: &str = "BACnetEventLogRecord timestamp";
const DATUM: &str = "BACnetEventLogRecord log-datum";

const LOG_STATUS: u8 = 0;
const NOTIFICATION: u8 = 1;
const TIME_CHANGE: u8 = 2;

/// Whether the log-datum CHOICE gives `tag` no alternative: above
/// time-change, the last it assigns.
fn unassigned(tag: u8) -> bool {
    tag > TIME_CHANGE
}

/// Encode one Event Log record.
///
/// Fails, leaving `buf` unchanged, for a notification that would not encode,
/// such as one whose raw event values aren't a well-formed run of tagged
/// fields, an opaque body that isn't one, or a record nested too deep for a
/// ReadRange-ACK to carry.
pub fn encode_event_log_record(
    record: &BACnetEventLogRecord,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    // Straight into `buf`, cut back on failure: no staging buffer (#2233).
    let start = buf.len();
    write_event_log_record(record, buf)
        .and_then(|()| check_record(&buf[start..], IN_ITEM_DATA, RECORD))
        .inspect_err(|_| buf.truncate(start))
}

fn write_event_log_record(record: &BACnetEventLogRecord, out: &mut BytesMut) -> Result<(), Error> {
    encode_timestamp(out, &record.date, &record.time);
    tags::encode_opening_tag(out, 1);
    match &record.log_datum {
        EventLogDatum::LogStatus(status) => encode_log_status(out, LOG_STATUS, *status),
        EventLogDatum::Notification(notification) => {
            tags::encode_opening_tag(out, NOTIFICATION);
            encode_event_notification(notification, out)?;
            tags::encode_closing_tag(out, NOTIFICATION);
        }
        EventLogDatum::TimeChange(seconds) => {
            primitives::encode_ctx_real(out, TIME_CHANGE, *seconds)
        }
        EventLogDatum::Opaque(opaque) => encode_opaque(out, opaque, unassigned, DATUM)?,
    }
    tags::encode_closing_tag(out, 1);
    Ok(())
}

/// Decode one Event Log record starting at `offset`, returning it and the
/// offset just past it.
///
/// A notification decodes in full, so a record whose notification isn't a
/// valid request fails as a whole. The one exception is a message text whose
/// characters don't decode, such as one in a character set this stack doesn't
/// support: the notification reads with no message text, as a received
/// notification does at the client ([`decode_event_notification_tolerant`]).
pub fn decode_event_log_record(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetEventLogRecord, usize), Error> {
    let (date, time, datum_start) = decode_timestamp(data, offset, TIMESTAMP)?;
    let (body, end) = decode_ctx_constructed(data, datum_start, 1, DATUM)?;
    let log_datum = decode_datum(body, datum_start)?;
    Ok((
        BACnetEventLogRecord {
            date,
            time,
            log_datum,
        },
        end,
    ))
}

fn decode_datum(data: &[u8], offset: usize) -> Result<EventLogDatum, Error> {
    let (tag, start) = tags::decode_tag(data, 0)?;
    let (datum, end) = if let Some((opaque, end)) = decode_opaque(data, &tag, start, unassigned)? {
        (EventLogDatum::Opaque(opaque), end)
    } else if tag.is_context(LOG_STATUS) {
        let (octets, end) = contents(data, start, tag.length)?;
        let status = decode_log_status(octets, offset, RECORD)?;
        (EventLogDatum::LogStatus(status), end)
    } else if tag.is_opening_tag(NOTIFICATION) {
        let (parameters, end) = tags::extract_context_value(data, start, NOTIFICATION)?;
        let notification = decode_event_notification_tolerant(parameters)?;
        (EventLogDatum::Notification(notification), end)
    } else if tag.is_context(TIME_CHANGE) {
        let (octets, end) = contents(data, start, tag.length)?;
        (
            EventLogDatum::TimeChange(primitives::decode_real(octets)?),
            end,
        )
    } else {
        return Err(Error::decoding(
            offset,
            "BACnetEventLogRecord log-datum expected context [0], constructed [1], or context [2]",
        ));
    };
    expect_end(data, end, offset, DATUM)?;
    Ok(datum)
}
