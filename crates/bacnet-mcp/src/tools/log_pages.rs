//! What the log tools share (`bacnet_read_trend` and
//! `bacnet_read_log_records`): the window arguments, the cursor, and the
//! paging loop over ReadRange, which stops at the call's record limit, its
//! structured-content cap or its budget and resumes by sequence number on
//! the next call.

use bacnet_client::log_reader::{LogCursor, LogPage};
use bacnet_ops::engine::{Engine, timed_out};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::trend_data::stamp_text;
use super::{MAX_STRUCTURED, next, schema, start};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;

/// The most records one call returns.
const MAX_RECORDS: u32 = 2_000;

/// The records a call returns unless asked otherwise.
const DEFAULT_RECORDS: u32 = 500;

/// The most records a summary call reads.
const MAX_SUMMARY: u32 = 20_000;

/// The records asked for in one ReadRange.
const PAGE: u32 = 200;

/// A log tool's input schema; `object` describes the logs it reads.
pub(super) fn input(object: &str) -> Value {
    let time = |what: &str| {
        json!({
            "type": "string",
            "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}(:[0-9]{2}(\\.[0-9]{1,2})?)?$",
            "description": format!("{what}, the device's local time: YYYY-MM-DDTHH:MM[:SS[.hh]]"),
        })
    };
    schema::input(
        json!({
            "device": schema::device(),
            "object": { "type": "string", "description": object },
            "from": time("Records logged after this time"),
            "to": time("Records logged up to this time"),
            "max_records": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RECORDS,
                "default": DEFAULT_RECORDS,
            },
            "summary": {
                "type": "boolean",
                "default": false,
                "description": "Read the whole window and return only the summary",
            },
            "cursor": schema::cursor(),
        }),
        &["device", "object"],
    )
}

/// The output fields every log tool's result has besides its records and
/// summary: how the reading went.
pub(super) fn paging_output(properties: &mut Map<String, Value>) {
    for (key, schema) in [
        (
            "pages",
            json!({ "type": "integer", "description": "ReadRange answers read" }),
        ),
        (
            "more_items",
            json!({
                "type": "integer",
                "description": "Answers the device cut short to fit (MORE_ITEMS)",
            }),
        ),
        (
            "gaps",
            json!({
                "type": "array",
                "description": "Records the log no longer held when read",
                "items": { "type": "object" },
            }),
        ),
        (
            "done",
            json!({
                "type": "boolean",
                "description": "The read reached the newest record or the end of the window",
            }),
        ),
        ("stopped", json!({ "type": "object" })),
    ] {
        properties.insert(key.into(), schema);
    }
}

/// A log tool's arguments.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    pub(super) device: DeviceArg,
    pub(super) object: String,
    from: Option<String>,
    to: Option<String>,
    max_records: Option<u32>,
    #[serde(default)]
    pub(super) summary: bool,
    cursor: Option<String>,
}

impl Args {
    /// Refuse a window that ends before it starts, as arguments outside
    /// the schema are refused (JSON-RPC -32602). Times that don't parse
    /// are left for the call, which names the field.
    pub(super) fn check(&self) -> Result<(), rmcp::ErrorData> {
        let at = |text: &Option<String>| {
            text.as_deref()
                .and_then(|text| parse_time("time", text).ok())
                .map(|(date, time)| stamp_text(&date, &time))
        };
        match (at(&self.from), at(&self.to)) {
            (Some(from), Some(to)) if from > to => Err(rmcp::ErrorData::invalid_params(
                format!("arguments: from ({from}) is after to ({to})"),
                None,
            )),
            _ => Ok(()),
        }
    }

    /// The log, when it is one of `kinds`; `what` names them for the
    /// refusal.
    pub(super) fn log(
        &self,
        kinds: &[ObjectType],
        what: &str,
    ) -> Result<ObjectIdentifier, ToolError> {
        let (kind, instance) = bacnet_ops::parse::parse_object_specifier(&self.object)
            .map_err(|e| ToolError::invalid("object", e))?;
        if !kinds.contains(&kind) {
            return Err(ToolError::invalid("object", what.to_string()));
        }
        ObjectIdentifier::new(kind, instance)
            .map_err(|e| ToolError::invalid("object", e.to_string()))
    }

    /// The window's end as stamp text, if it has one.
    pub(super) fn to(&self) -> Result<Option<String>, ToolError> {
        self.to
            .as_deref()
            .map(|text| parse_time("to", text).map(|(d, t)| stamp_text(&d, &t)))
            .transpose()
    }

    /// The records the call reads at most.
    pub(super) fn limit(&self) -> Result<u32, ToolError> {
        if self.summary {
            return Ok(MAX_SUMMARY);
        }
        let wanted = self.max_records.unwrap_or(DEFAULT_RECORDS);
        if !(1..=MAX_RECORDS).contains(&wanted) {
            return Err(ToolError::invalid(
                "max_records",
                format!("1 to {MAX_RECORDS}"),
            ));
        }
        Ok(wanted)
    }

    /// Where the read starts: the cursor's sequence number, or the window's
    /// start, or the oldest record.
    pub(super) fn cursor<T: TransportPort + 'static>(
        &self,
        ctx: &Ctx<T>,
        tool: &str,
        raw: &Map<String, Value>,
    ) -> Result<LogCursor, ToolError> {
        let from = self
            .from
            .as_deref()
            .map(|text| parse_time("from", text))
            .transpose()?;
        Ok(match self.cursor.as_deref() {
            Some(_) => LogCursor::Sequence(start(ctx, tool, self.cursor.as_deref(), raw)?),
            None => from.map_or(LogCursor::Oldest, |(date, time)| {
                LogCursor::Time(date, time)
            }),
        })
    }
}

/// A local date and time argument.
fn parse_time(field: &'static str, text: &str) -> Result<(Date, Time), ToolError> {
    let bad = || ToolError::invalid(field, format!("'{text}' is not YYYY-MM-DDTHH:MM[:SS[.hh]]"));
    let (date, time) = text.split_once(['T', ' ']).ok_or_else(bad)?;
    let mut ymd = date.splitn(3, '-').map(str::parse::<u16>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) = (ymd.next(), ymd.next(), ymd.next())
    else {
        return Err(bad());
    };
    let (clock, hundredths) = match time.split_once('.') {
        Some((clock, fraction)) => {
            let digits: u8 = fraction.parse().map_err(|_| bad())?;
            (
                clock,
                if fraction.len() == 1 {
                    digits * 10
                } else {
                    digits
                },
            )
        }
        None => (time, 0),
    };
    let mut hms = clock.splitn(3, ':').map(str::parse::<u8>);
    let (Some(Ok(hour)), Some(Ok(minute))) = (hms.next(), hms.next()) else {
        return Err(bad());
    };
    let second = match hms.next() {
        Some(Ok(second)) => second,
        Some(Err(_)) => return Err(bad()),
        None => 0,
    };
    let days = days_in(year, month).ok_or_else(bad)?;
    if !(1900..=2154).contains(&year)
        || day == 0
        || u32::from(day) > days
        || hour > 23
        || minute > 59
        || second > 59
        || hundredths > 99
    {
        return Err(bad());
    }
    let date = Date {
        year: u8::try_from(year - 1900).map_err(|_| bad())?,
        month: u8::try_from(month).map_err(|_| bad())?,
        day: u8::try_from(day).map_err(|_| bad())?,
        day_of_week: weekday(year, month, day),
    };
    let time = Time {
        hour,
        minute,
        second,
        hundredths,
    };
    Ok((date, time))
}

/// Days in `month` of `year`, or `None` for no such month.
fn days_in(year: u16, month: u16) -> Option<u32> {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

/// The day of the week, 1 for Monday to 7 for Sunday (Sakamoto's method).
fn weekday(year: u16, month: u16, day: u16) -> u8 {
    const OFFSETS: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = u32::from(if month < 3 { year - 1 } else { year });
    let sunday_zero =
        (y + y / 4 - y / 100 + y / 400 + OFFSETS[usize::from(month - 1)] + u32::from(day)) % 7;
    // 0 is Sunday here; BACnet counts Monday as 1.
    u8::try_from(if sunday_zero == 0 { 7 } else { sunday_zero }).unwrap_or(7)
}

/// A log cursor as a cursor position: the next sequence number. Every
/// page these tools read goes by sequence number (from the oldest record
/// or a time, then on by number), so nothing else needs resuming.
fn pack(cursor: LogCursor) -> Option<u64> {
    match cursor {
        LogCursor::Sequence(seq) => Some(seq),
        _ => None,
    }
}

/// The top of `log`'s sequence numbers: an Audit Log counts in 64 bits,
/// the other logs in 32 (Clauses 12.25.16, 12.27.15 and 12.64.12).
fn top(log: ObjectIdentifier) -> u64 {
    if log.object_type() == ObjectType::AUDIT_LOG {
        u64::MAX
    } else {
        u64::from(u32::MAX)
    }
}

/// The sequence number `n` records after `first` in a log numbered from 1
/// up to `top` and round again.
fn advance(first: u64, n: u64, top: u64) -> u64 {
    if first == 0 {
        return n;
    }
    // In 128 bits: an Audit Log's numbers reach the top of 64.
    let at = (u128::from(first) - 1 + u128::from(n)) % u128::from(top) + 1;
    u64::try_from(at).unwrap_or(u64::MAX)
}

/// One call's reading of a log: the pages read, the records taken, and
/// where the next call resumes.
pub(super) struct Pager {
    log: ObjectIdentifier,
    device: u32,
    cursor: LogCursor,
    limit: u32,
    /// The records taken.
    pub(super) count: u32,
    /// The rows' size so far, against [`MAX_STRUCTURED`].
    size: usize,
    rows: Vec<Value>,
    /// The ReadRange answers read.
    pub(super) pages: u32,
    more_items: u32,
    gaps: Vec<Value>,
    done: bool,
    stopped: Option<Value>,
    resume: Option<LogCursor>,
}

impl Pager {
    /// A reading of `log` on `device` from `cursor`, of at most `limit`
    /// records.
    pub(super) fn new(device: u32, log: ObjectIdentifier, cursor: LogCursor, limit: u32) -> Self {
        Self {
            log,
            device,
            cursor,
            limit,
            count: 0,
            size: 0,
            rows: Vec::new(),
            pages: 0,
            more_items: 0,
            gaps: Vec::new(),
            done: false,
            stopped: None,
            resume: None,
        }
    }

    fn full(&self) -> bool {
        self.count >= self.limit || self.size >= MAX_STRUCTURED
    }

    /// The next page, or `None` once the reading stops: the limit or the
    /// structured-content cap reached, the budget spent, or a later page
    /// failed (kept as `stopped`). A first page that fails fails the call.
    pub(super) async fn next<T: TransportPort + 'static>(
        &mut self,
        engine: &Engine<T>,
        call: &Call,
    ) -> Result<Option<LogPage>, ToolError> {
        if self.full() || (self.pages > 0 && call.out_of_time()) {
            self.resume = Some(self.cursor);
            return Ok(None);
        }
        let want = u16::try_from((self.limit - self.count).min(PAGE)).unwrap_or(1);
        let page = match engine
            .read_trend(self.device, self.log, self.cursor, want)
            .await
        {
            Ok(page) => page,
            Err(error) if self.pages == 0 => return Err(ToolError::device(self.device, &error)),
            Err(error) => {
                if timed_out(&error) {
                    call.timed_out();
                }
                self.stopped = Some(ToolError::device(self.device, &error).json());
                self.resume = Some(self.cursor);
                return Ok(None);
            }
        };
        self.pages += 1;
        if page.result_flags.2 {
            self.more_items += 1;
        }
        if let Some(gap) = page.gap {
            self.gaps.push(json!({
                "expected": gap.expected,
                "first": gap.first,
                "skipped": gap.skipped,
            }));
        }
        Ok(Some(page))
    }

    /// Record `i` of `page`, logged at `time`: `Some` with its sequence
    /// number (when the log numbered it) to take it, or `None` when the
    /// reading ends before it, past the window's end `to` or full.
    pub(super) fn admit(
        &mut self,
        page: &LogPage,
        i: usize,
        time: &str,
        to: Option<&str>,
    ) -> Option<Option<u64>> {
        let n = u64::try_from(i).unwrap_or(u64::MAX);
        let seq = page
            .first_sequence_number
            .map(|first| advance(first, n, top(self.log)));
        if to.is_some_and(|to| time > to) {
            self.done = true;
            return None;
        }
        if self.full() {
            self.resume = Some(match seq {
                Some(seq) => LogCursor::Sequence(seq),
                // A page without sequence numbers: go on after it.
                None => page.next,
            });
            return None;
        }
        self.count += 1;
        Some(seq)
    }

    /// Keep a taken record's row.
    pub(super) fn push(&mut self, row: Value) {
        self.size += row.to_string().len();
        self.rows.push(row);
    }

    /// After `page`'s records: whether to read another page.
    pub(super) fn read_on(&mut self, page: &LogPage) -> bool {
        if self.done || self.resume.is_some() {
            return false;
        }
        self.cursor = page.next;
        if page.done {
            self.done = true;
            return false;
        }
        true
    }

    /// Put the reading into `body`: the records, how the reading went, and
    /// the cursor of `tool` for the rest.
    pub(super) fn finish<T: TransportPort + 'static>(
        mut self,
        ctx: &Ctx<T>,
        tool: &str,
        raw: &Map<String, Value>,
        body: &mut Value,
    ) {
        let resume = self.resume.and_then(pack);
        if self.resume.is_some() && resume.is_none() && self.stopped.is_none() {
            // More remains, but at no sequence number a cursor can carry.
            self.stopped = Some(json!({
                "error": "failed",
                "message": "the log has more records, but gave no sequence number to resume from",
            }));
        }
        body["records"] = json!(self.rows);
        body["pages"] = json!(self.pages);
        body["more_items"] = json!(self.more_items);
        body["gaps"] = json!(self.gaps);
        body["done"] = json!(self.done);
        body["truncated"] = json!(self.resume.is_some());
        body["next_cursor"] = json!(resume.map(|at| next(ctx, tool, raw, at)));
        body["limits"] = ctx.limits(Some(usize::try_from(self.limit).unwrap_or(usize::MAX)));
        if let Some(stopped) = self.stopped {
            body["stopped"] = stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_with_their_weekday() {
        let (date, time) = parse_time("from", "2026-10-25T07:30").unwrap();
        assert_eq!((date.year, date.month, date.day), (126, 10, 25));
        // 25 October 2026 is a Sunday.
        assert_eq!(date.day_of_week, 7);
        assert_eq!((time.hour, time.minute, time.second), (7, 30, 0));
        let (date, time) = parse_time("to", "2024-02-29 23:59:59.5").unwrap();
        assert_eq!(date.day_of_week, 4, "a Thursday");
        assert_eq!(time.hundredths, 50);
        for bad in [
            "2026-02-30T00:00",
            "2026-13-01T00:00",
            "2026-10-25T24:00",
            "2026-10-25",
            "1899-01-01T00:00",
            "2026-10-25T07:00:00.123",
        ] {
            assert!(parse_time("from", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cursors_pack_and_sequences_wrap_at_their_log_top() {
        assert_eq!(pack(LogCursor::Sequence(17)), Some(17));
        assert_eq!(pack(LogCursor::Oldest), None);
        assert_eq!(pack(LogCursor::Position(3)), None);

        let trend = u64::from(u32::MAX);
        assert_eq!(advance(5, 3, trend), 8);
        assert_eq!(advance(trend, 1, trend), 1);
        assert_eq!(advance(0, 2, trend), 2);
        // An Audit Log's numbers pass 2^32 - 1 and wrap at the top of 64.
        assert_eq!(advance(trend, 1, u64::MAX), trend + 1);
        assert_eq!(advance(u64::MAX, 1, u64::MAX), 1);
        let audit = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap();
        let events = ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap();
        assert_eq!((top(audit), top(events)), (u64::MAX, trend));
    }
}
