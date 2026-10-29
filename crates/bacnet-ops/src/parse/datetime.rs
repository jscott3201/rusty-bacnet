//! Dates, times, date-times and timestamps in the forms the formatter
//! prints them, so a value read can be typed back: `2026-10-19 Mon`,
//! `*-even-months-last-day`, `07:00`, `07:00:05.25`, `2026-10-19 07:00`,
//! `any date`, `any time`, `unspecified` and `seq 42`.

use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

/// Lowest and highest full year a Date holds; 255 above 1900 means
/// unspecified.
const YEARS: std::ops::RangeInclusive<u16> = 1900..=2154;

const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// A weekday by name, `Mon` or `monday` in any case, as its number from
/// Monday 1.
fn weekday(word: &str) -> Option<u8> {
    let word = word.to_ascii_lowercase();
    if word.len() < 3 {
        return None;
    }
    WEEKDAYS
        .iter()
        .position(|day| day.starts_with(&word))
        .map(|index| index as u8 + 1)
}

/// One numeric field, or `*` for unspecified.
fn field(name: &str, text: &str, range: std::ops::RangeInclusive<u8>) -> Result<u8, String> {
    if text == "*" {
        return Ok(Date::UNSPECIFIED);
    }
    match text.parse::<u8>() {
        Ok(value) if range.contains(&value) => Ok(value),
        _ => Err(format!(
            "invalid {name} '{text}': {}-{} or * for any",
            range.start(),
            range.end()
        )),
    }
}

/// Whether `text` starts like a date: four digits or `*`, then a hyphen.
pub(super) fn looks_like_date(text: &str) -> bool {
    let year = text.split('-').next().unwrap_or_default();
    text.contains('-')
        && (year == "*" || (year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit())))
}

/// Whether `text` looks like a time: digits or `*` around a colon.
pub(super) fn looks_like_time(text: &str) -> bool {
    !text.is_empty()
        && text.contains(':')
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || b"*:.".contains(&b))
}

/// The `YYYY-MM-DD` part of a date, special months and days by name.
fn ymd(text: &str) -> Result<Date, String> {
    let invalid = || {
        format!(
            "invalid date '{text}': expected YYYY-MM-DD, * for any field, \
             odd-months/even-months, last-day/odd-days/even-days"
        )
    };
    let (year, rest) = text.split_once('-').ok_or_else(invalid)?;
    let year = if year == "*" {
        Date::UNSPECIFIED
    } else {
        let full: u16 = year.parse().map_err(|_| invalid())?;
        if !YEARS.contains(&full) {
            return Err(format!("invalid year {full}: 1900-2154 or * for any"));
        }
        (full - 1900) as u8
    };
    let lower = rest.to_ascii_lowercase();
    let (month, day) = if let Some(day) = lower.strip_prefix("odd-months-") {
        (13, day)
    } else if let Some(day) = lower.strip_prefix("even-months-") {
        (14, day)
    } else {
        let (month, day) = lower.split_once('-').ok_or_else(invalid)?;
        (field("month", month, 1..=14)?, day)
    };
    let day = match day {
        "last-day" => 32,
        "odd-days" => 33,
        "even-days" => 34,
        day => field("day", day, 1..=34)?,
    };
    Ok(Date {
        year,
        month,
        day,
        day_of_week: Date::UNSPECIFIED,
    })
}

/// A time: `HH:MM`, `HH:MM:SS` or `HH:MM:SS.hh`, `*` for any field, or
/// `any time`. Fields left off are 0.
pub fn parse_time(text: &str) -> Result<Time, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("any time") {
        return Ok(Time {
            hour: Time::UNSPECIFIED,
            minute: Time::UNSPECIFIED,
            second: Time::UNSPECIFIED,
            hundredths: Time::UNSPECIFIED,
        });
    }
    let (clock, hundredths) = match text.split_once('.') {
        Some((clock, hundredths)) => (clock, Some(hundredths)),
        None => (text, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour, minute] => (*hour, *minute, None),
        [hour, minute, second] => (*hour, *minute, Some(*second)),
        _ => {
            return Err(format!(
                "invalid time '{text}': expected HH:MM, HH:MM:SS or HH:MM:SS.hh"
            ));
        }
    };
    if hundredths.is_some() && second.is_none() {
        return Err(format!(
            "invalid time '{text}': hundredths need seconds, HH:MM:SS.hh"
        ));
    }
    // Two digits, as the formatter prints them: `.5` could mean 5 or 50.
    if let Some(h) = hundredths
        && h != "*"
        && h.len() != 2
    {
        return Err(format!(
            "invalid hundredths '{h}' in '{text}': two digits, .05 or .50, or * for any"
        ));
    }
    Ok(Time {
        hour: field("hour", hour, 0..=23)?,
        minute: field("minute", minute, 0..=59)?,
        second: second.map_or(Ok(0), |s| field("second", s, 0..=59))?,
        hundredths: hundredths.map_or(Ok(0), |h| field("hundredths", h, 0..=99))?,
    })
}

/// The words of a date or date-time, with `any date` and `any time` kept
/// as one word each.
fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let joins = matches!(out.last(), Some(last) if last.eq_ignore_ascii_case("any"))
            && (word.eq_ignore_ascii_case("date") || word.eq_ignore_ascii_case("time"));
        match out.last_mut() {
            Some(last) if joins => {
                last.push(' ');
                last.push_str(word);
            }
            _ => out.push(word.to_string()),
        }
    }
    out
}

/// A date and maybe a time after it, as the formatter writes them:
/// `2026-10-19`, `2026-10-19 Mon 07:00`, `any date 07:00`, or
/// `unspecified` for neither. The time is `None` when none was typed.
pub fn parse_date_time(text: &str) -> Result<(Date, Option<Time>), String> {
    let text = text.trim();
    let any_date = Date {
        year: Date::UNSPECIFIED,
        month: Date::UNSPECIFIED,
        day: Date::UNSPECIFIED,
        day_of_week: Date::UNSPECIFIED,
    };
    if text.eq_ignore_ascii_case("unspecified") {
        return Ok((any_date, Some(parse_time("any time")?)));
    }
    let words = words(text);
    let mut rest = words.iter().map(String::as_str).peekable();
    let mut date = match rest.next() {
        Some(word) if word.eq_ignore_ascii_case("any date") => any_date,
        Some(word) => ymd(word)?,
        None => return Err("expected a date, YYYY-MM-DD".into()),
    };
    if let Some(day) = rest.peek().and_then(|word| weekday(word)) {
        date.day_of_week = day;
        rest.next();
    }
    let time = match rest.next() {
        None => None,
        Some(word) => Some(parse_time(word)?),
    };
    if let Some(extra) = rest.next() {
        return Err(format!("unexpected '{extra}' after the date and time"));
    }
    Ok((date, time))
}

/// A date with no time after it: `2026-10-19`, `2026-10-19 Mon`,
/// `*-*-* Fri`, `any date`.
pub fn parse_date(text: &str) -> Result<Date, String> {
    match parse_date_time(text)? {
        (date, None) => Ok(date),
        (_, Some(_)) => Err(format!(
            "'{}' has a time; a Date takes only YYYY-MM-DD and a weekday",
            text.trim()
        )),
    }
}

/// A BACnetTimeStamp as the formatter prints one: `seq 42`, a time
/// (`14:30:05`, `any time`), or a date and time (`2026-10-25 Sun 14:30:05`;
/// a date alone leaves the time unspecified).
pub fn parse_timestamp(text: &str) -> Result<BACnetTimeStamp, String> {
    let text = text.trim();
    if let Some(number) = text
        .strip_prefix("seq ")
        .or_else(|| text.strip_prefix("sequence "))
    {
        let number = number.trim();
        return number
            .parse::<u16>()
            .map(BACnetTimeStamp::SequenceNumber)
            .map_err(|_| format!("invalid sequence number '{number}': 0-65535"));
    }
    if looks_like_time(text) || text.eq_ignore_ascii_case("any time") {
        return parse_time(text).map(BACnetTimeStamp::Time);
    }
    let (date, time) = parse_date_time(text).map_err(|e| {
        format!("{e}; a timestamp is `seq N`, a time `HH:MM[:SS[.hh]]` or a date and time")
    })?;
    Ok(BACnetTimeStamp::DateTime {
        date,
        time: match time {
            Some(time) => time,
            None => parse_time("any time")?,
        },
    })
}
