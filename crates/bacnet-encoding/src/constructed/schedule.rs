//! Clause 21 codecs for the Schedule object's constructed properties
//! (Clause 12.24): Weekly_Schedule and Exception_Schedule.
//!
//! - `BACnetTimeValue` is a SEQUENCE of two untagged members, so it is an
//!   application-tagged Time followed by one application-tagged primitive
//!   value, with no frame. [`BACnetTimeValue::value`] holds that value typed.
//! - `BACnetDailySchedule` is a SEQUENCE with one member, day-schedule `[0]`,
//!   a SEQUENCE OF `BACnetTimeValue`: an opening/closing `[0]` pair around the
//!   time-values. Weekly_Schedule is a BACnetARRAY\[7\] of them, Monday first.
//! - `BACnetSpecialEvent` is three members with no outer frame:
//!
//! | Member              | Tag          | Encoding                                         |
//! |---------------------|--------------|--------------------------------------------------|
//! | period              | (untagged)   | CHOICE: calendar-entry `[0]` or calendar-reference `[1]` |
//! | list-of-time-values | `[2]`        | opening/closing pair around the time-values      |
//! | event-priority      | `[3]`        | primitive Unsigned, 1 through 16 when valid      |
//!
//! The calendar-entry alternative wraps a `BACnetCalendarEntry`, itself a
//! CHOICE, so it is an opening/closing `[0]` pair around the entry's own
//! tagged encoding (see [`super::calendar`]). The calendar-reference
//! alternative is a primitive `[1]` Calendar object identifier.
//! Exception_Schedule is a BACnetARRAY of special events, concatenated when
//! read whole.
//!
//! These codecs check structure only, like the calendar-entry codec: a
//! well-formed event with a priority outside 1 through 16 decodes, and the
//! consumer range-checks it (#1087).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::Time;
use bytes::BytesMut;

use super::calendar::{decode_calendar_entry, encode_calendar_entry};
use super::tagged::{
    contents, decode_ctx_object_id, decode_ctx_unsigned, expect_closing, expect_end,
    expect_opening, next_is_closing,
};
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, TagClass};

/// Encode one `BACnetTimeValue`: the application-tagged Time, then the
/// application-tagged value.
///
/// Refuses a value that is not of a primitive datatype (a `List` or
/// `ApplicationData`), which no time-value can carry.
pub fn encode_time_value(buf: &mut BytesMut, time_value: &BACnetTimeValue) -> Result<(), Error> {
    if !time_value.value.is_primitive() {
        return Err(Error::encode("time-value: the value must be of a primitive datatype"));
    }
    primitives::encode_app_time(buf, &time_value.time);
    primitives::encode_property_value(buf, &time_value.value)
}

/// Decode one `BACnetTimeValue` at `offset`.
///
/// The value must be a single application-tagged primitive. Returns the
/// time-value and the offset just past it.
pub fn decode_time_value(data: &[u8], offset: usize) -> Result<(BACnetTimeValue, usize), Error> {
    let (tag, content) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::TIME || tag.length != 4 {
        return Err(Error::decoding(
            offset,
            "time-value: expected an application-tagged Time (4 octets)",
        ));
    }
    let (octets, value_start) = contents(data, content, 4)?;
    let time = Time::decode(octets)?;

    let (tag, _) = tags::decode_tag(data, value_start)?;
    if tag.class != TagClass::Application {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            value_start,
            "time-value: expected an application-tagged value, got context tag [{}]",
        )
        .with_value(tag.number.into())
        .into());
    }
    let (value, value_end) = primitives::decode_application_value(data, value_start)?;
    Ok((BACnetTimeValue { time, value }, value_end))
}

/// Encode a SEQUENCE OF `BACnetTimeValue` inside an opening/closing `tag` pair.
fn encode_time_values(
    buf: &mut BytesMut,
    tag: u8,
    time_values: &[BACnetTimeValue],
) -> Result<(), Error> {
    tags::encode_opening_tag(buf, tag);
    for time_value in time_values {
        encode_time_value(buf, time_value)?;
    }
    tags::encode_closing_tag(buf, tag);
    Ok(())
}

/// Decode a SEQUENCE OF `BACnetTimeValue` framed by an opening/closing `tag`
/// pair at `offset`; returns the time-values and the offset past the closing tag.
fn decode_time_values(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(Vec<BACnetTimeValue>, usize), Error> {
    let mut pos = expect_opening(data, offset, tag, what)?;
    let mut time_values = Vec::new();
    loop {
        if next_is_closing(data, pos, tag)? {
            return Ok((time_values, expect_closing(data, pos, tag, what)?));
        }
        if time_values.len() >= MAX_FRAMED_ITEMS {
            return Err(DecodeError::new(
                DecodingKind::Overflow,
                pos,
                "time-values exceed item limit",
            )
            .within(what)
            .into());
        }
        let (time_value, next) = decode_time_value(data, pos)?;
        time_values.push(time_value);
        pos = next;
    }
}

/// Encode the period CHOICE of a `BACnetSpecialEvent`.
pub fn encode_special_event_period(buf: &mut BytesMut, period: &SpecialEventPeriod) {
    match period {
        SpecialEventPeriod::CalendarEntry(entry) => {
            tags::encode_opening_tag(buf, 0);
            encode_calendar_entry(buf, entry);
            tags::encode_closing_tag(buf, 0);
        }
        SpecialEventPeriod::CalendarReference(oid) => primitives::encode_ctx_object_id(buf, 1, oid),
    }
}

/// Decode the period CHOICE of a `BACnetSpecialEvent` at `offset`.
pub fn decode_special_event_period(
    data: &[u8],
    offset: usize,
) -> Result<(SpecialEventPeriod, usize), Error> {
    let (tag, content) = tags::decode_tag(data, offset)?;
    if tag.is_opening_tag(0) {
        let (entry, after) = decode_calendar_entry(data, content)?;
        let end = expect_closing(data, after, 0, "special event calendar-entry")?;
        return Ok((SpecialEventPeriod::CalendarEntry(entry), end));
    }
    if tag.is_context(1) {
        let (oid, end) = decode_ctx_object_id(data, offset, 1, "special event calendar-reference")?;
        return Ok((SpecialEventPeriod::CalendarReference(oid), end));
    }
    Err(Error::decoding(
        offset,
        "special event period requires calendar-entry [0] or calendar-reference [1]",
    ))
}

/// Encode one `BACnetSpecialEvent`; fails only on a time-value that
/// [`encode_time_value`] refuses. The event-priority goes out as held, in or
/// out of its range.
pub fn encode_special_event(buf: &mut BytesMut, event: &BACnetSpecialEvent) -> Result<(), Error> {
    encode_special_event_period(buf, &event.period);
    encode_time_values(buf, 2, &event.list_of_time_values)?;
    primitives::encode_ctx_unsigned(buf, 3, event.event_priority);
    Ok(())
}

/// Decode one `BACnetSpecialEvent` at `offset`; returns the event and the
/// offset just past it.
///
/// The event-priority decodes as any Unsigned. Its 1 through 16 range is a
/// property of the value, not of the encoding, so a consumer that stores or
/// acts on the event checks it: the Schedule object refuses a priority out of
/// range with VALUE_OUT_OF_RANGE (Clause 15.9.1.3), as it does for an inline
/// calendar entry holding an octet out of range.
pub fn decode_special_event(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetSpecialEvent, usize), Error> {
    let (period, pos) = decode_special_event_period(data, offset)?;
    let (list_of_time_values, pos) =
        decode_time_values(data, pos, 2, "special event list-of-time-values")?;
    let (event_priority, end) =
        decode_ctx_unsigned::<u64>(data, pos, 3, "special event event-priority")?;
    Ok((
        BACnetSpecialEvent {
            period,
            list_of_time_values,
            event_priority,
        },
        end,
    ))
}

/// Encode one `BACnetDailySchedule`: its time-values inside `[0]`. Fails only
/// on a time-value that [`encode_time_value`] refuses.
pub fn encode_daily_schedule(
    buf: &mut BytesMut,
    time_values: &[BACnetTimeValue],
) -> Result<(), Error> {
    encode_time_values(buf, 0, time_values)
}

/// Decode one `BACnetDailySchedule` at `offset`; returns its time-values and
/// the offset just past the closing `[0]`.
pub fn decode_daily_schedule(
    data: &[u8],
    offset: usize,
) -> Result<(Vec<BACnetTimeValue>, usize), Error> {
    decode_time_values(data, offset, 0, "daily schedule")
}

/// Encode a whole Weekly_Schedule: seven daily schedules, Monday first.
pub fn encode_weekly_schedule(
    buf: &mut BytesMut,
    days: &[Vec<BACnetTimeValue>; 7],
) -> Result<(), Error> {
    for day in days {
        encode_daily_schedule(buf, day)?;
    }
    Ok(())
}

/// The refusal of a schedule's entry `index` at `offset`, named by `what`:
/// the inner reader's fault, restated there without formatting (#1776).
#[cold]
fn entry_refusal(error: &Error, offset: usize, what: &'static str, index: u16) -> Error {
    error
        .to_decode_error(offset)
        .within(what)
        .at_item(index)
        .into()
}

/// Decode a whole Weekly_Schedule.
///
/// The data must hold exactly seven daily schedules and nothing after them:
/// the property is a fixed-size BACnetARRAY\[7\].
pub fn decode_weekly_schedule(data: &[u8]) -> Result<[Vec<BACnetTimeValue>; 7], Error> {
    let mut days: [Vec<BACnetTimeValue>; 7] = Default::default();
    let mut pos = 0;
    for (index, day) in (0u16..).zip(days.iter_mut()) {
        let (time_values, next) = decode_daily_schedule(data, pos)
            .map_err(|error| entry_refusal(&error, pos, "weekly schedule day", index))?;
        *day = time_values;
        pos = next;
    }
    expect_end(data, pos, pos, "weekly schedule after 7 daily schedules")?;
    Ok(days)
}

/// Encode a whole Exception_Schedule: the special events, concatenated.
pub fn encode_exception_schedule(
    buf: &mut BytesMut,
    events: &[BACnetSpecialEvent],
) -> Result<(), Error> {
    for event in events {
        encode_special_event(buf, event)?;
    }
    Ok(())
}

/// Decode a whole Exception_Schedule: zero or more special events, back to
/// back, with every byte belonging to one. Like [`decode_special_event`], it
/// leaves each event-priority's range to the caller.
pub fn decode_exception_schedule(data: &[u8]) -> Result<Vec<BACnetSpecialEvent>, Error> {
    let mut events = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if events.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                pos,
                "exception schedule exceeds item limit",
            ));
        }
        // The item limit keeps the count within u16.
        let index = u16::try_from(events.len()).unwrap_or(u16::MAX);
        let (event, next) = decode_special_event(data, pos)
            .map_err(|error| entry_refusal(&error, pos, "exception schedule entry", index))?;
        events.push(event);
        pos = next;
    }
    Ok(events)
}
