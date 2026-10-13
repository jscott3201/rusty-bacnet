//! Clause 21 codecs for `BACnetCalendarEntry` and `BACnetDateRange`.
//!
//! A calendar entry is the element of the Calendar object's Date_List
//! (Clause 12.9) and the inline period of a Schedule `BACnetSpecialEvent`
//! (Clause 12.24). It is a CHOICE whose alternatives carry their own context
//! tags:
//!
//! | Alternative | Tag   | Encoding                                          |
//! |-------------|-------|---------------------------------------------------|
//! | date        | `[0]` | primitive, the four Date octets                   |
//! | date-range  | `[1]` | opening/closing pair around a `BACnetDateRange`   |
//! | weekNDay    | `[2]` | primitive, the three `BACnetWeekNDay` octets      |
//!
//! `BACnetDateRange` is a SEQUENCE of two untagged Dates, so each is
//! application-tagged: start date, then end date. Unframed, as a Schedule's
//! Effective_Period carries it, that pair is the whole encoding. A BACnetLIST
//! of calendar entries is the plain concatenation of its elements.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay};
use bacnet_types::error::Error;
use bacnet_types::primitives::Date;
use bytes::BytesMut;

use super::tagged::{contents, expect_closing};
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, TagClass};

/// Encode one unframed `BACnetDateRange`: its start and end application Dates.
pub fn encode_date_range(buf: &mut BytesMut, range: &BACnetDateRange) {
    primitives::encode_app_date(buf, &range.start_date);
    primitives::encode_app_date(buf, &range.end_date);
}

/// Decode one unframed `BACnetDateRange` at `offset`.
///
/// Returns the range and the offset just past it. Both members must be
/// application-tagged Dates of exactly four octets.
pub fn decode_date_range(data: &[u8], offset: usize) -> Result<(BACnetDateRange, usize), Error> {
    let (start_date, offset) = decode_app_date(data, offset, "date range start-date")?;
    let (end_date, offset) = decode_app_date(data, offset, "date range end-date")?;
    Ok((
        BACnetDateRange {
            start_date,
            end_date,
        },
        offset,
    ))
}

/// Encode one `BACnetCalendarEntry` under its CHOICE tag.
pub fn encode_calendar_entry(buf: &mut BytesMut, entry: &BACnetCalendarEntry) {
    match entry {
        BACnetCalendarEntry::Date(date) => primitives::encode_ctx_date(buf, 0, date),
        BACnetCalendarEntry::DateRange(range) => {
            tags::encode_opening_tag(buf, 1);
            encode_date_range(buf, range);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetCalendarEntry::WeekNDay(week_n_day) => {
            primitives::encode_ctx_octet_string(buf, 2, &week_n_day.encode());
        }
    }
}

/// Decode one `BACnetCalendarEntry` at `offset`.
///
/// Returns the entry and the offset just past it. Fails with
/// [`Error::Decoding`] or [`Error::BufferTooShort`] when the leading tag is
/// not one of the three context tags (an application-tagged Date or Octet
/// String included), a primitive alternative has the wrong length, the
/// date-range frame holds anything but two application Dates or is not
/// closed by `[1]`, or the data ends early.
pub fn decode_calendar_entry(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetCalendarEntry, usize), Error> {
    let (tag, content) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) {
        let (octets, end) = fixed_content(data, offset, content, tag.length, 4, "date [0]")?;
        return Ok((BACnetCalendarEntry::Date(Date::decode(octets)?), end));
    }
    if tag.is_opening_tag(1) {
        let (range, after) = decode_date_range(data, content)?;
        let end = expect_closing(data, after, 1, "calendar entry date-range")?;
        return Ok((BACnetCalendarEntry::DateRange(range), end));
    }
    if tag.is_context(2) {
        let (octets, end) = fixed_content(data, offset, content, tag.length, 3, "weekNDay [2]")?;
        return Ok((
            BACnetCalendarEntry::WeekNDay(BACnetWeekNDay::decode(octets)?),
            end,
        ));
    }
    Err(Error::decoding(
        offset,
        "calendar entry requires date [0], date-range [1] or weekNDay [2]",
    ))
}

/// Encode a BACnetLIST of `BACnetCalendarEntry` as concatenated elements.
pub fn encode_calendar_entry_list(buf: &mut BytesMut, entries: &[BACnetCalendarEntry]) {
    for entry in entries {
        encode_calendar_entry(buf, entry);
    }
}

/// Decode a complete BACnetLIST of `BACnetCalendarEntry`.
///
/// Every byte must belong to an entry; an empty input is an empty list.
pub fn decode_calendar_entry_list(data: &[u8]) -> Result<Vec<BACnetCalendarEntry>, Error> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        if entries.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                "calendar entry list exceeds item limit",
            ));
        }
        let (entry, end) = decode_calendar_entry(data, offset)?;
        entries.push(entry);
        offset = end;
    }
    Ok(entries)
}

/// Decode one application-tagged Date of exactly four octets.
fn decode_app_date(data: &[u8], offset: usize, what: &str) -> Result<(Date, usize), Error> {
    let (tag, content) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::DATE {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected an application-tagged Date"),
        ));
    }
    let (octets, end) = fixed_content(data, offset, content, tag.length, 4, what)?;
    Ok((Date::decode(octets)?, end))
}

/// The content octets of a primitive tag that must hold exactly `expected`
/// octets, and the offset just past them.
fn fixed_content<'a>(
    data: &'a [u8],
    offset: usize,
    content: usize,
    length: u32,
    expected: usize,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    if usize::try_from(length).ok() != Some(expected) {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected {expected} content octets, got {length}"),
        ));
    }
    contents(data, content, length)
}
