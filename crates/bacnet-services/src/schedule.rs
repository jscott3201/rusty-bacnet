//! Schedule-property codecs per ASHRAE 135-2020 Clauses 12.17, 21.
//!
//! These cover the wire-format encode/decode for the constructed types that
//! the Schedule object's `weekly-schedule` and `exception-schedule` carry:
//!
//! - [`BACnetTimeValue`]               — `(Time, application-tagged value)`
//! - [`BACnetCalendarEntry`]           — `CHOICE { Date, DateRange, WeekNDay }`
//! - [`SpecialEventPeriod`]            — `CHOICE { CalendarEntry, CalendarReference }`
//! - [`BACnetSpecialEvent`]            — full exception-schedule entry
//!
//! And the two top-level array convenience helpers:
//!
//! - [`encode_weekly_schedule`] / [`decode_weekly_schedule`]
//!   `BACnetARRAY[7] OF BACnetDailySchedule`
//! - [`encode_exception_schedule`] / [`decode_exception_schedule`]
//!   `SEQUENCE OF BACnetSpecialEvent`
//!
//! `BACnetTimeValue.value` stays as `Vec<u8>` — the raw application-tagged
//! bytes for a single primitive datatype. Consumers that want a typed
//! [`PropertyValue`] back can run [`decode_application_value`] on those
//! bytes themselves.
//!
//! [`PropertyValue`]: bacnet_types::primitives::PropertyValue
//! [`decode_application_value`]: bacnet_encoding::primitives::decode_application_value

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// BACnetTimeValue
// ---------------------------------------------------------------------------
//
// BACnetTimeValue ::= SEQUENCE {
//     time   Time,                      -- application-tagged
//     value  ABSTRACT-SYNTAX.&Type      -- application-tagged primitive
// }
//
// Both fields are application-tagged, so the encoding is just concatenation
// of two application-tagged values with no outer wrapper.

/// Encode a single `BACnetTimeValue` (application-tagged Time + raw
/// application-tagged value bytes).
pub fn encode_time_value(buf: &mut BytesMut, tv: &BACnetTimeValue) {
    primitives::encode_app_time(buf, &tv.time);
    buf.extend_from_slice(&tv.value);
}

/// Decode a single `BACnetTimeValue` starting at `offset`. Returns the
/// decoded value and the offset past the consumed bytes.
pub fn decode_time_value(data: &[u8], offset: usize) -> Result<(BACnetTimeValue, usize), Error> {
    // Application-tagged Time, length 4.
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application
        || tag.number != tags::app_tag::TIME
        || tag.length != 4
    {
        return Err(Error::decoding(
            offset,
            "TimeValue: expected application-tagged Time (4 bytes)",
        ));
    }
    let end = pos
        .checked_add(4)
        .ok_or_else(|| Error::decoding(pos, "TimeValue: time length overflow"))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let time = Time::decode(&data[pos..end])?;

    // One application-tagged value — slice out tag header + content as raw
    // bytes so the BACnetTimeValue.value field stays opaque.
    let value_start = end;
    let (val_tag, val_pos) = tags::decode_tag(data, value_start)?;
    if val_tag.class != tags::TagClass::Application {
        return Err(Error::decoding(
            value_start,
            format!(
                "TimeValue: expected application tag for value, got context tag {}",
                val_tag.number
            ),
        ));
    }
    if val_tag.is_opening || val_tag.is_closing {
        return Err(Error::decoding(
            value_start,
            "TimeValue: unexpected opening/closing tag in value",
        ));
    }
    // Booleans encode their payload in the L/V/T field with no content
    // octets — every other primitive uses the tag length. This matches the
    // dispatch in decode_application_value.
    let content_len = if val_tag.number == tags::app_tag::BOOLEAN {
        0
    } else {
        val_tag.length as usize
    };
    let value_end = val_pos
        .checked_add(content_len)
        .ok_or_else(|| Error::decoding(val_pos, "TimeValue: value length overflow"))?;
    if value_end > data.len() {
        return Err(Error::buffer_too_short(value_end, data.len()));
    }

    Ok((
        BACnetTimeValue {
            time,
            value: data[value_start..value_end].to_vec(),
        },
        value_end,
    ))
}

// ---------------------------------------------------------------------------
// BACnetCalendarEntry
// ---------------------------------------------------------------------------
//
// BACnetCalendarEntry ::= CHOICE {
//     date         [0] Date,
//     date-range   [1] BACnetDateRange,
//     weekNDay     [2] BACnetWeekNDay
// }
//
// Date and WeekNDay are primitive — encoded as ctx-tagged content.
// DateRange is a SEQUENCE — encoded with [1] opening / closing wrappers
// around its two application-tagged Date fields.

/// Encode a `BACnetCalendarEntry` (one of three CHOICE variants).
pub fn encode_calendar_entry(buf: &mut BytesMut, e: &BACnetCalendarEntry) {
    match e {
        BACnetCalendarEntry::Date(d) => {
            primitives::encode_ctx_date(buf, 0, d);
        }
        BACnetCalendarEntry::DateRange(dr) => {
            tags::encode_opening_tag(buf, 1);
            primitives::encode_app_date(buf, &dr.start_date);
            primitives::encode_app_date(buf, &dr.end_date);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetCalendarEntry::WeekNDay(w) => {
            primitives::encode_ctx_octet_string(buf, 2, &w.encode());
        }
    }
}

/// Decode a `BACnetCalendarEntry` starting at `offset`.
pub fn decode_calendar_entry(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetCalendarEntry, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Context {
        return Err(Error::decoding(
            offset,
            "CalendarEntry: expected context tag",
        ));
    }

    match tag.number {
        0 => {
            // [0] Date — primitive, length 4.
            if tag.length != 4 {
                return Err(Error::decoding(
                    offset,
                    format!("CalendarEntry::Date: expected length 4, got {}", tag.length),
                ));
            }
            let end = pos + 4;
            if end > data.len() {
                return Err(Error::buffer_too_short(end, data.len()));
            }
            let date = Date::decode(&data[pos..end])?;
            Ok((BACnetCalendarEntry::Date(date), end))
        }
        1 => {
            // [1] DateRange — opening / two app-Dates / closing.
            if !tag.is_opening {
                return Err(Error::decoding(
                    offset,
                    "CalendarEntry::DateRange: expected [1] opening tag",
                ));
            }
            let (start_date, p1) = read_app_date(data, pos)?;
            let (end_date, p2) = read_app_date(data, p1)?;
            let (close, p3) = tags::decode_tag(data, p2)?;
            if !close.is_closing_tag(1) {
                return Err(Error::decoding(
                    p2,
                    "CalendarEntry::DateRange: expected [1] closing tag",
                ));
            }
            Ok((
                BACnetCalendarEntry::DateRange(BACnetDateRange {
                    start_date,
                    end_date,
                }),
                p3,
            ))
        }
        2 => {
            // [2] WeekNDay — primitive octet string, length 3.
            if tag.length != 3 {
                return Err(Error::decoding(
                    offset,
                    format!(
                        "CalendarEntry::WeekNDay: expected length 3, got {}",
                        tag.length
                    ),
                ));
            }
            let end = pos + 3;
            if end > data.len() {
                return Err(Error::buffer_too_short(end, data.len()));
            }
            let w = BACnetWeekNDay::decode(&data[pos..end])?;
            Ok((BACnetCalendarEntry::WeekNDay(w), end))
        }
        other => Err(Error::decoding(
            offset,
            format!("CalendarEntry: unknown CHOICE tag [{other}]"),
        )),
    }
}

// ---------------------------------------------------------------------------
// SpecialEventPeriod
// ---------------------------------------------------------------------------
//
// (the period field of BACnetSpecialEvent)
// CHOICE {
//     calendar-entry     [0] BACnetCalendarEntry,
//     calendar-reference [1] BACnetObjectIdentifier
// }
//
// CalendarEntry is itself a CHOICE that uses context tags — wrapped in [0]
// opening / closing because its content is constructed.
// CalendarReference is a primitive ObjectIdentifier — directly ctx-tagged.

/// Encode a `SpecialEventPeriod`.
pub fn encode_special_event_period(buf: &mut BytesMut, p: &SpecialEventPeriod) {
    match p {
        SpecialEventPeriod::CalendarEntry(e) => {
            tags::encode_opening_tag(buf, 0);
            encode_calendar_entry(buf, e);
            tags::encode_closing_tag(buf, 0);
        }
        SpecialEventPeriod::CalendarReference(oid) => {
            primitives::encode_ctx_object_id(buf, 1, oid);
        }
    }
}

/// Decode a `SpecialEventPeriod` starting at `offset`.
pub fn decode_special_event_period(
    data: &[u8],
    offset: usize,
) -> Result<(SpecialEventPeriod, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Context {
        return Err(Error::decoding(offset, "Period: expected context tag"));
    }

    match tag.number {
        0 => {
            if !tag.is_opening {
                return Err(Error::decoding(
                    offset,
                    "Period::CalendarEntry: expected [0] opening tag",
                ));
            }
            let (entry, p1) = decode_calendar_entry(data, pos)?;
            let (close, p2) = tags::decode_tag(data, p1)?;
            if !close.is_closing_tag(0) {
                return Err(Error::decoding(
                    p1,
                    "Period::CalendarEntry: expected [0] closing tag",
                ));
            }
            Ok((SpecialEventPeriod::CalendarEntry(entry), p2))
        }
        1 => {
            if tag.length != 4 {
                return Err(Error::decoding(
                    offset,
                    format!(
                        "Period::CalendarReference: expected length 4, got {}",
                        tag.length
                    ),
                ));
            }
            let end = pos + 4;
            if end > data.len() {
                return Err(Error::buffer_too_short(end, data.len()));
            }
            let oid = ObjectIdentifier::decode(&data[pos..end])?;
            Ok((SpecialEventPeriod::CalendarReference(oid), end))
        }
        other => Err(Error::decoding(
            offset,
            format!("Period: unknown CHOICE tag [{other}]"),
        )),
    }
}

// ---------------------------------------------------------------------------
// BACnetSpecialEvent
// ---------------------------------------------------------------------------
//
// BACnetSpecialEvent ::= SEQUENCE {
//     period CHOICE { calendar-entry [0] ..., calendar-reference [1] ... },
//     list-of-time-values [2] SEQUENCE OF BACnetTimeValue,
//     event-priority      [3] Unsigned (1..16)
// }

/// Encode a `BACnetSpecialEvent` (period + list-of-time-values + priority).
pub fn encode_special_event(buf: &mut BytesMut, e: &BACnetSpecialEvent) {
    encode_special_event_period(buf, &e.period);

    // [2] list-of-time-values
    tags::encode_opening_tag(buf, 2);
    for tv in &e.list_of_time_values {
        encode_time_value(buf, tv);
    }
    tags::encode_closing_tag(buf, 2);

    // [3] event-priority
    primitives::encode_ctx_unsigned(buf, 3, e.event_priority as u64);
}

/// Decode a `BACnetSpecialEvent` starting at `offset`.
pub fn decode_special_event(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetSpecialEvent, usize), Error> {
    let (period, mut pos) = decode_special_event_period(data, offset)?;

    // [2] list-of-time-values opening
    let (open, p1) = tags::decode_tag(data, pos)?;
    if !open.is_opening_tag(2) {
        return Err(Error::decoding(
            pos,
            "SpecialEvent: expected [2] opening tag for list-of-time-values",
        ));
    }
    pos = p1;

    let mut list_of_time_values = Vec::new();
    loop {
        let (peek, _) = tags::decode_tag(data, pos)?;
        if peek.is_closing_tag(2) {
            break;
        }
        if list_of_time_values.len() >= MAX_DECODED_ITEMS {
            return Err(Error::decoding(
                pos,
                "SpecialEvent: list-of-time-values exceeds MAX_DECODED_ITEMS",
            ));
        }
        let (tv, next) = decode_time_value(data, pos)?;
        list_of_time_values.push(tv);
        pos = next;
    }
    // consume the closing tag
    let (_close, p2) = tags::decode_tag(data, pos)?;
    pos = p2;

    // [3] event-priority
    let (prio_tag, p3) = tags::decode_tag(data, pos)?;
    if !prio_tag.is_context(3) {
        return Err(Error::decoding(
            pos,
            "SpecialEvent: expected [3] event-priority",
        ));
    }
    let prio_end = p3 + prio_tag.length as usize;
    if prio_end > data.len() {
        return Err(Error::buffer_too_short(prio_end, data.len()));
    }
    let prio = primitives::decode_unsigned(&data[p3..prio_end])?;
    let event_priority = validate_priority(prio, pos)?;

    Ok((
        BACnetSpecialEvent {
            period,
            list_of_time_values,
            event_priority,
        },
        prio_end,
    ))
}

/// Validate the event-priority field per Clause 21 (Unsigned 1..16).
///
/// We accept the raw value at decode time but reject anything outside
/// 1..=16 because that's spec-required and a wider value almost always
/// means a malformed payload (or a different field landed in this slot).
/// Erroring here gives the caller a precise location instead of letting a
/// 0 or 99 silently flow into Schedule's priority resolution.
fn validate_priority(raw: u64, offset: usize) -> Result<u8, Error> {
    if !(1..=16).contains(&raw) {
        return Err(Error::decoding(
            offset,
            format!("SpecialEvent: event-priority {raw} outside 1..=16"),
        ));
    }
    Ok(raw as u8)
}

// ---------------------------------------------------------------------------
// weekly-schedule (BACnetARRAY[7] OF BACnetDailySchedule)
// ---------------------------------------------------------------------------
//
// When read as the whole property (no array index), the ARRAY[7] is encoded
// as 7 BACnetDailySchedule values back-to-back. Each BACnetDailySchedule is
// a SEQUENCE with one [0]-tagged field:
//
//     BACnetDailySchedule ::= SEQUENCE { day-schedule [0] SEQUENCE OF BACnetTimeValue }
//
// On the wire that's `[0]opening` ... `[0]closing` per day.

/// Encode a 7-day weekly schedule (`BACnetARRAY[7] OF BACnetDailySchedule`).
///
/// Mon..Sun in array order, matching the BACnet day-of-week numbering
/// (1=Mon..7=Sun) — `days[0]` is Monday's TimeValue list.
pub fn encode_weekly_schedule(buf: &mut BytesMut, days: &[Vec<BACnetTimeValue>; 7]) {
    for day in days {
        tags::encode_opening_tag(buf, 0);
        for tv in day {
            encode_time_value(buf, tv);
        }
        tags::encode_closing_tag(buf, 0);
    }
}

/// Decode a 7-day weekly schedule.
///
/// Errors if the payload doesn't contain exactly 7 daily schedules — the
/// spec requires a fixed-size ARRAY[7] and a count mismatch is the
/// signature of a malformed or truncated response.
pub fn decode_weekly_schedule(data: &[u8]) -> Result<[Vec<BACnetTimeValue>; 7], Error> {
    let mut days: [Vec<BACnetTimeValue>; 7] = Default::default();
    let mut pos = 0usize;
    for (i, day) in days.iter_mut().enumerate() {
        let (open, p1) = tags::decode_tag(data, pos)
            .map_err(|e| Error::decoding(pos, format!("WeeklySchedule day {i}: {e}")))?;
        if !open.is_opening_tag(0) {
            return Err(Error::decoding(
                pos,
                format!("WeeklySchedule day {i}: expected [0] opening tag"),
            ));
        }
        pos = p1;

        loop {
            let (peek, _) = tags::decode_tag(data, pos)
                .map_err(|e| Error::decoding(pos, format!("WeeklySchedule day {i}: {e}")))?;
            if peek.is_closing_tag(0) {
                break;
            }
            if day.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    pos,
                    format!("WeeklySchedule day {i}: exceeds MAX_DECODED_ITEMS"),
                ));
            }
            let (tv, next) = decode_time_value(data, pos)
                .map_err(|e| Error::decoding(pos, format!("WeeklySchedule day {i}: {e}")))?;
            day.push(tv);
            pos = next;
        }
        let (_close, p2) = tags::decode_tag(data, pos)
            .map_err(|e| Error::decoding(pos, format!("WeeklySchedule day {i}: {e}")))?;
        pos = p2;
    }

    if pos != data.len() {
        return Err(Error::decoding(
            pos,
            format!(
                "WeeklySchedule: {} trailing byte(s) after 7 daily schedules",
                data.len() - pos
            ),
        ));
    }
    Ok(days)
}

// ---------------------------------------------------------------------------
// exception-schedule (SEQUENCE OF BACnetSpecialEvent)
// ---------------------------------------------------------------------------

/// Encode the exception-schedule property value (concatenated SpecialEvents).
pub fn encode_exception_schedule(buf: &mut BytesMut, events: &[BACnetSpecialEvent]) {
    for e in events {
        encode_special_event(buf, e);
    }
}

/// Decode the exception-schedule property value (zero or more SpecialEvents
/// back-to-back).
pub fn decode_exception_schedule(data: &[u8]) -> Result<Vec<BACnetSpecialEvent>, Error> {
    let mut events = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        if events.len() >= MAX_DECODED_ITEMS {
            return Err(Error::decoding(
                pos,
                "ExceptionSchedule: exceeds MAX_DECODED_ITEMS",
            ));
        }
        let (event, next) = decode_special_event(data, pos).map_err(|e| {
            Error::decoding(
                pos,
                format!("ExceptionSchedule entry {}: {e}", events.len()),
            )
        })?;
        events.push(event);
        pos = next;
    }
    Ok(events)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn read_app_date(data: &[u8], offset: usize) -> Result<(Date, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application
        || tag.number != tags::app_tag::DATE
        || tag.length != 4
    {
        return Err(Error::decoding(
            offset,
            "expected application-tagged Date (4 bytes)",
        ));
    }
    let end = pos + 4;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((Date::decode(&data[pos..end])?, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn d(year_offset: u8, month: u8, day: u8, dow: u8) -> Date {
        Date {
            year: year_offset,
            month,
            day,
            day_of_week: dow,
        }
    }

    fn t(h: u8, m: u8, s: u8) -> Time {
        Time {
            hour: h,
            minute: m,
            second: s,
            hundredths: 0,
        }
    }

    /// Helper: build a BACnetTimeValue with an application-Real value.
    fn tv_real(hour: u8, minute: u8, value: f32) -> BACnetTimeValue {
        let mut buf = BytesMut::new();
        primitives::encode_app_real(&mut buf, value);
        BACnetTimeValue {
            time: t(hour, minute, 0),
            value: buf.to_vec(),
        }
    }

    /// Helper: build a BACnetTimeValue with an application-Null value.
    fn tv_null(hour: u8, minute: u8) -> BACnetTimeValue {
        let mut buf = BytesMut::new();
        primitives::encode_app_null(&mut buf);
        BACnetTimeValue {
            time: t(hour, minute, 0),
            value: buf.to_vec(),
        }
    }

    // --- BACnetTimeValue ---------------------------------------------------

    #[test]
    fn time_value_round_trip_real() {
        let tv = tv_real(8, 30, 72.5);
        let mut buf = BytesMut::new();
        encode_time_value(&mut buf, &tv);
        let (decoded, end) = decode_time_value(&buf, 0).unwrap();
        assert_eq!(decoded, tv);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn time_value_round_trip_null() {
        let tv = tv_null(17, 0);
        let mut buf = BytesMut::new();
        encode_time_value(&mut buf, &tv);
        let (decoded, end) = decode_time_value(&buf, 0).unwrap();
        assert_eq!(decoded, tv);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn time_value_rejects_context_tagged_value() {
        let mut buf = BytesMut::new();
        primitives::encode_app_time(&mut buf, &t(8, 0, 0));
        primitives::encode_ctx_unsigned(&mut buf, 0, 1);
        let err = decode_time_value(&buf, 0).unwrap_err();
        assert!(format!("{err}").contains("expected application tag"));
    }

    // --- BACnetCalendarEntry ----------------------------------------------

    #[test]
    fn calendar_entry_date_round_trip() {
        let e = BACnetCalendarEntry::Date(d(124, 7, 4, 4));
        let mut buf = BytesMut::new();
        encode_calendar_entry(&mut buf, &e);
        let (decoded, end) = decode_calendar_entry(&buf, 0).unwrap();
        assert_eq!(decoded, e);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn calendar_entry_date_range_round_trip() {
        let e = BACnetCalendarEntry::DateRange(BACnetDateRange {
            start_date: d(124, 1, 1, 1),
            end_date: d(124, 12, 31, 2),
        });
        let mut buf = BytesMut::new();
        encode_calendar_entry(&mut buf, &e);
        let (decoded, end) = decode_calendar_entry(&buf, 0).unwrap();
        assert_eq!(decoded, e);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn calendar_entry_week_n_day_round_trip() {
        let e = BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: 0xFF,
            week_of_month: 1,
            day_of_week: 1,
        });
        let mut buf = BytesMut::new();
        encode_calendar_entry(&mut buf, &e);
        let (decoded, end) = decode_calendar_entry(&buf, 0).unwrap();
        assert_eq!(decoded, e);
        assert_eq!(end, buf.len());
    }

    // --- SpecialEventPeriod -----------------------------------------------

    #[test]
    fn period_calendar_entry_round_trip() {
        let p = SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(d(124, 12, 25, 3)));
        let mut buf = BytesMut::new();
        encode_special_event_period(&mut buf, &p);
        let (decoded, end) = decode_special_event_period(&buf, 0).unwrap();
        assert_eq!(decoded, p);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn period_calendar_reference_round_trip() {
        let oid = ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap();
        let p = SpecialEventPeriod::CalendarReference(oid);
        let mut buf = BytesMut::new();
        encode_special_event_period(&mut buf, &p);
        let (decoded, end) = decode_special_event_period(&buf, 0).unwrap();
        assert_eq!(decoded, p);
        assert_eq!(end, buf.len());
    }

    // --- BACnetSpecialEvent ------------------------------------------------

    #[test]
    fn special_event_round_trip_with_calendar_entry() {
        let e = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: 11,
                    week_of_month: 4,
                    day_of_week: 4,
                },
            )),
            list_of_time_values: vec![tv_real(0, 0, 1.0), tv_null(23, 59)],
            event_priority: 5,
        };
        let mut buf = BytesMut::new();
        encode_special_event(&mut buf, &e);
        let (decoded, end) = decode_special_event(&buf, 0).unwrap();
        assert_eq!(decoded, e);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn special_event_round_trip_with_calendar_reference() {
        let e = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(
                ObjectIdentifier::new(ObjectType::CALENDAR, 7).unwrap(),
            ),
            list_of_time_values: vec![tv_real(8, 0, 70.0), tv_real(18, 0, 60.0)],
            event_priority: 16,
        };
        let mut buf = BytesMut::new();
        encode_special_event(&mut buf, &e);
        let (decoded, end) = decode_special_event(&buf, 0).unwrap();
        assert_eq!(decoded, e);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn special_event_priority_zero_is_rejected() {
        // Build a payload with a deliberately invalid priority of 0.
        let mut buf = BytesMut::new();
        encode_special_event_period(
            &mut buf,
            &SpecialEventPeriod::CalendarReference(
                ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap(),
            ),
        );
        tags::encode_opening_tag(&mut buf, 2);
        tags::encode_closing_tag(&mut buf, 2);
        primitives::encode_ctx_unsigned(&mut buf, 3, 0);

        let err = decode_special_event(&buf, 0).unwrap_err();
        assert!(format!("{err}").contains("event-priority 0"));
    }

    #[test]
    fn special_event_priority_seventeen_is_rejected() {
        let mut buf = BytesMut::new();
        encode_special_event_period(
            &mut buf,
            &SpecialEventPeriod::CalendarReference(
                ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap(),
            ),
        );
        tags::encode_opening_tag(&mut buf, 2);
        tags::encode_closing_tag(&mut buf, 2);
        primitives::encode_ctx_unsigned(&mut buf, 3, 17);

        let err = decode_special_event(&buf, 0).unwrap_err();
        assert!(format!("{err}").contains("event-priority 17"));
    }

    // --- weekly-schedule ---------------------------------------------------

    #[test]
    fn weekly_schedule_empty_round_trip() {
        let days: [Vec<BACnetTimeValue>; 7] = Default::default();
        let mut buf = BytesMut::new();
        encode_weekly_schedule(&mut buf, &days);
        let decoded = decode_weekly_schedule(&buf).unwrap();
        assert_eq!(decoded, days);
        // Empty schedule = 7 pairs of opening/closing tags = 14 bytes.
        assert_eq!(buf.len(), 14);
    }

    #[test]
    fn weekly_schedule_partially_populated_round_trip() {
        // Monday: heat to 70 at 6am, setback to 65 at 10pm. Wed: only 8am setpoint.
        let mut days: [Vec<BACnetTimeValue>; 7] = Default::default();
        days[0] = vec![tv_real(6, 0, 70.0), tv_real(22, 0, 65.0)];
        days[2] = vec![tv_real(8, 0, 72.0)];
        let mut buf = BytesMut::new();
        encode_weekly_schedule(&mut buf, &days);
        let decoded = decode_weekly_schedule(&buf).unwrap();
        assert_eq!(decoded, days);
    }

    #[test]
    fn weekly_schedule_rejects_six_days() {
        // Truncated: only 6 daily schedules instead of 7.
        let mut buf = BytesMut::new();
        for _ in 0..6 {
            tags::encode_opening_tag(&mut buf, 0);
            tags::encode_closing_tag(&mut buf, 0);
        }
        let err = decode_weekly_schedule(&buf).unwrap_err();
        assert!(format!("{err}").contains("day 6"));
    }

    #[test]
    fn weekly_schedule_rejects_trailing_bytes() {
        let days: [Vec<BACnetTimeValue>; 7] = Default::default();
        let mut buf = BytesMut::new();
        encode_weekly_schedule(&mut buf, &days);
        buf.extend_from_slice(&[0xAA, 0xBB]); // garbage past the schedule
        let err = decode_weekly_schedule(&buf).unwrap_err();
        assert!(format!("{err}").contains("trailing byte"));
    }

    // --- exception-schedule ------------------------------------------------

    #[test]
    fn exception_schedule_empty_round_trip() {
        let events: Vec<BACnetSpecialEvent> = Vec::new();
        let mut buf = BytesMut::new();
        encode_exception_schedule(&mut buf, &events);
        assert!(buf.is_empty());
        let decoded = decode_exception_schedule(&buf).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn exception_schedule_two_events_round_trip() {
        let events = vec![
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(d(
                    124, 12, 25, 3,
                ))),
                list_of_time_values: vec![tv_real(0, 0, 1.0)],
                event_priority: 1,
            },
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarReference(
                    ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap(),
                ),
                list_of_time_values: vec![tv_null(0, 0), tv_real(12, 0, 70.0)],
                event_priority: 8,
            },
        ];
        let mut buf = BytesMut::new();
        encode_exception_schedule(&mut buf, &events);
        let decoded = decode_exception_schedule(&buf).unwrap();
        assert_eq!(decoded, events);
    }
}
