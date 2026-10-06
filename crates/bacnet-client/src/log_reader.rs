//! Paged reads of a log's Log_Buffer over ReadRange (#1530).
//!
//! [`read_log_page`] reads one page from a cursor and returns the cursor to
//! read from next, so a caller loops until the page says it is done and
//! keeps the last cursor as its checkpoint. One request is outstanding at a
//! time. On a [`BACnetClient`] the client's minimum request interval paces
//! them, and on the endpoint client the session's does (#1542).
//!
//! Sequence numbers are the log's own: a log numbers each record with its
//! Total_Record_Count once the record is added, so the newest record carries
//! Total_Record_Count and the oldest that number less Record_Count, plus
//! one. The numbers count up to the top of their Unsigned32 range
//! (Unsigned64 for an Audit Log) and carry on from 1, never 0 (Clauses
//! 12.25.14 and 12.25.16).

use core::future::Future;

use bacnet_encoding::constructed::tagged::{decode_app_unsigned, expect_end};
use bacnet_services::read_range::{
    LogRecords, RangeSpec, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
    ReadRangeViolation,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use crate::client::BACnetClient;

/// Where a paged log read starts, or resumes from a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogCursor {
    /// The oldest record the log holds, found from its Record_Count and
    /// Total_Record_Count; the read goes on by sequence number.
    Oldest,
    /// The record with this sequence number, then on by sequence number.
    Sequence(u64),
    /// The record at this one-based position, then on by position. The
    /// fallback for a device whose sequence numbers are inconsistent, which
    /// fails a sequence read with [`Error::LogNotAdvancing`]. Not for a log
    /// that is merely full: each record it drops shifts every position, so a
    /// position read of a busy full log skips records without a gap.
    Position(u64),
    /// The first record newer than this date and time, which must be
    /// specific; the read goes on by sequence number.
    Time(Date, Time),
}

/// A page whose first record is not the one the read asked for: the log no
/// longer holds the records in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogGap {
    /// The sequence number the read asked for.
    pub expected: u64,
    /// The sequence number of the page's first record.
    pub first: u64,
    /// How many records were lost between the two; `None` when the log's
    /// numbering went back below `expected`, as when its Total_Record_Count
    /// was reset.
    pub skipped: Option<u64>,
}

/// One page of a paged log read.
#[derive(Debug, Clone)]
pub struct LogPage {
    /// The page's records, oldest first.
    pub records: LogRecords,
    /// The first record's sequence number; `None` for a page read by
    /// position, or an empty page.
    pub first_sequence_number: Option<u64>,
    /// FIRST_ITEM, LAST_ITEM and MORE_ITEMS as the device set them; all
    /// false for a page the reader made up for an empty log.
    pub result_flags: (bool, bool, bool),
    /// Set when the first record is past the one the read asked for.
    pub gap: Option<LogGap>,
    /// Rules the device's answer broke that the reader tolerates, such as a
    /// first sequence number of 0 after the device's wrap.
    pub violations: Vec<ReadRangeViolation>,
    /// Where to read next: the next page while `done` is false, else the
    /// checkpoint to resume from once the log has more records.
    pub next: LogCursor,
    /// The page reached the newest record (LAST_ITEM), or found none.
    pub done: bool,
    /// The page's records reach the top of the sequence range, so `next`
    /// wrapped to 1. A device that numbers the record after the top 0
    /// rather than 1 (against the standard) is then one number ahead of
    /// `next`; see [`read_log_page`].
    pub wrapped: bool,
}

/// What a paged log read needs from a requester: a property read as an
/// Unsigned, and a ReadRange checked leniently, so the reader decides which
/// broken rules to tolerate.
pub trait LogRequester {
    /// Read `property` of `log` at `mac` as an Unsigned.
    fn read_unsigned(
        &self,
        mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> impl Future<Output = Result<u64, Error>> + Send;

    /// Send one ReadRange to `mac` and check its answer leniently.
    fn read_range_lenient(
        &self,
        mac: &[u8],
        request: &ReadRangeRequest,
    ) -> impl Future<Output = Result<ReadRangeReply, Error>> + Send;
}

impl<T: TransportPort + 'static> LogRequester for BACnetClient<T> {
    async fn read_unsigned(
        &self,
        mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let ack = self.read_property(mac, log, property, None).await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        self.read_range_with(mac, request, ReadRangeValidation::Lenient)
            .await
    }
}

/// An application-tagged Unsigned that fills `value`.
pub fn unsigned_value(value: &[u8]) -> Result<u64, Error> {
    const WHAT: &str = "log record count";
    let (count, end) = decode_app_unsigned::<u64>(value, 0, WHAT)?;
    expect_end(value, end, end, WHAT)?;
    Ok(count)
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read one page of `log`'s Log_Buffer from `cursor`, asking for up to
    /// `page_size` records (1..=32767); see [`read_log_page`].
    ///
    /// ```no_run
    /// # use bacnet_client::client::BACnetClient;
    /// # use bacnet_client::log_reader::LogCursor;
    /// # use bacnet_transport::bip::BipTransport;
    /// # async fn read_all(
    /// #     client: &BACnetClient<BipTransport>,
    /// #     mac: &[u8],
    /// #     log: bacnet_types::primitives::ObjectIdentifier,
    /// # ) -> Result<LogCursor, bacnet_types::error::Error> {
    /// let mut cursor = LogCursor::Oldest;
    /// loop {
    ///     let page = client.read_log_page(mac, log, cursor, 100).await?;
    ///     // ... store page.records ...
    ///     cursor = page.next;
    ///     if page.done {
    ///         return Ok(cursor); // persist as the checkpoint
    ///     }
    /// }
    /// # }
    /// ```
    pub async fn read_log_page(
        &self,
        mac: &[u8],
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        read_log_page(self, mac, log, cursor, page_size).await
    }
}

/// The sequence numbers a log of one type assigns: 1 up to `max`, then 1
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SequenceSpace {
    max: u64,
}

/// The records a log holds now, from its two counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    /// No records; the next one will carry `next`.
    Empty { next: u64 },
    /// Records from `oldest` to `newest`.
    Records { oldest: u64, newest: u64 },
}

impl SequenceSpace {
    /// The space of `object_type`'s log, or an error when it keeps no log.
    pub(crate) fn of(object_type: ObjectType) -> Result<Self, Error> {
        match object_type {
            ObjectType::AUDIT_LOG => Ok(Self { max: u64::MAX }),
            ObjectType::TREND_LOG | ObjectType::EVENT_LOG | ObjectType::TREND_LOG_MULTIPLE => {
                Ok(Self {
                    max: u64::from(u32::MAX),
                })
            }
            other => Err(Error::Encoding(format!(
                "a paged log read needs a Trend Log, Event Log, Trend Log Multiple or \
                 Audit Log, not {other}"
            ))),
        }
    }

    /// `sequence` moved on by `n` records. A device that numbered a record
    /// 0 after its wrap counts on from it in its own numbering.
    pub(crate) fn advance(self, sequence: u64, n: u64) -> u64 {
        if sequence == 0 {
            return n;
        }
        let size = u128::from(self.max);
        ((u128::from(sequence - 1) + u128::from(n)) % size + 1) as u64
    }

    /// The sequence number `back` records before `sequence`. After a
    /// Total_Record_Count of 0, the device counts through 0, so the
    /// arithmetic does too.
    fn retreat(self, sequence: u64, back: u64) -> u64 {
        if sequence == 0 {
            let size = u128::from(self.max) + 1;
            return ((size - u128::from(back) % size) % size) as u64;
        }
        let size = u128::from(self.max);
        ((u128::from(sequence - 1) + size - u128::from(back) % size) % size + 1) as u64
    }

    /// How far `to` lies past `from`: positive ahead, negative behind,
    /// across the wrap. 0, a device's number for the record after its wrap,
    /// counts as 1.
    pub(crate) fn distance(self, from: u64, to: u64) -> i128 {
        let size = i128::from(self.max);
        let at = |sequence: u64| i128::from(sequence.max(1));
        let ahead = (at(to) - at(from)).rem_euclid(size);
        if ahead > size / 2 {
            ahead - size
        } else {
            ahead
        }
    }

    /// The records a log holds, from its Record_Count and
    /// Total_Record_Count.
    pub(crate) fn window(self, record_count: u64, total: u64) -> Result<Window, Error> {
        if record_count > self.max || total > self.max {
            return Err(Error::OutOfRange(format!(
                "log counts {record_count} and {total} exceed the log's sequence range"
            )));
        }
        if record_count == 0 {
            return Ok(Window::Empty {
                next: self.advance(total, 1),
            });
        }
        Ok(Window::Records {
            oldest: self.retreat(total, record_count - 1),
            newest: total,
        })
    }
}

/// The ReadRange count for `page_size`.
fn page_count(page_size: u16) -> Result<i32, Error> {
    match i16::try_from(page_size) {
        Ok(count @ 1..) => Ok(i32::from(count)),
        _ => Err(Error::Encoding(format!(
            "log page size must be 1..=32767, got {page_size}"
        ))),
    }
}

fn request(log: ObjectIdentifier, cursor: LogCursor, count: i32) -> ReadRangeRequest {
    let range = match cursor {
        LogCursor::Oldest => unreachable!("Oldest resolves to a sequence number first"),
        LogCursor::Sequence(reference_seq) => RangeSpec::BySequenceNumber {
            reference_seq,
            count,
        },
        LogCursor::Position(reference_index) => RangeSpec::ByPosition {
            reference_index,
            count,
        },
        LogCursor::Time(date, time) => RangeSpec::ByTime {
            reference_time: (date, time),
            count,
        },
    };
    ReadRangeRequest {
        object_identifier: log,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(range),
    }
}

/// Most times a read reads the counts again because the log moved between
/// two requests, or starts over from the oldest record because the log
/// dropped it meanwhile.
const RETRIES: usize = 3;

/// The records `log` holds now.
///
/// Total_Record_Count is read on both sides of Record_Count: a record logged
/// in between would otherwise make the oldest look one newer than it is, and
/// the read would skip it without a gap. When the total moves, the counts are
/// read again, up to [`RETRIES`] times; after that the count goes with the
/// total read before it, which can only make the oldest look older. On a log
/// that isn't full that may report a gap no record fell into, but no record is
/// ever skipped without one; the empty-page path in [`read_log_page`]
/// recovers from a number the log doesn't hold.
async fn window<R: LogRequester + ?Sized>(
    requester: &R,
    mac: &[u8],
    log: ObjectIdentifier,
    space: SequenceSpace,
) -> Result<Window, Error> {
    let total = || requester.read_unsigned(mac, log, PropertyIdentifier::TOTAL_RECORD_COUNT);
    let mut before = total().await?;
    let mut attempts = 0;
    loop {
        let record_count = requester
            .read_unsigned(mac, log, PropertyIdentifier::RECORD_COUNT)
            .await?;
        let after = total().await?;
        attempts += 1;
        if after == before || attempts > RETRIES {
            return space.window(record_count, before);
        }
        before = after;
    }
}

/// Read one page of `log`'s Log_Buffer from `cursor` through `requester`,
/// asking for up to `page_size` records (1..=32767).
///
/// - [`LogCursor::Oldest`] reads Total_Record_Count, Record_Count and
///   Total_Record_Count again first to find the oldest record.
/// - Each page asks for `page_size` records from the cursor; the next page
///   starts at the first sequence number plus the records returned, across
///   the wrap. MORE_ITEMS only says the answer was cut short to fit, so any
///   page that isn't the last continues; LAST_ITEM or an empty page ends.
/// - A page whose first record is past the one asked for reports a
///   [`LogGap`]. An empty page by sequence number reads the counts again:
///   past the newest record the read is done; when the log holds the record
///   asked for (logged since), the read asks again; when it no longer holds
///   it, the read starts over from the oldest it does hold, with a gap. A
///   full log can drop that one too before it is read, so a read starts over
///   up to three times.
/// - A page that starts before the record asked for fails with
///   [`Error::LogNotAdvancing`] instead of repeating records. So does an
///   empty answer for a record the counts still say the log holds, once the
///   read has asked again for a record logged since, or after any restart
///   from the oldest; and a log that drops its oldest record faster than the
///   restarts catch it. A device that does either because its sequence
///   numbers are inconsistent can be read from [`LogCursor::Position`].
/// - The answer is checked leniently: a first sequence number of 0 (a
///   device's number for the record after its wrap), a first sequence number
///   on a page by position, contradictory flags and a count overrun are
///   tolerated and listed in [`LogPage::violations`]. Any other broken rule
///   fails with [`Error::ReadRangeViolation`].
/// - A page whose records don't decode fails with [`Error::Decoding`] naming
///   the first that fails; the records before it are dropped. Read that range
///   with `read_range_with` and decode it with
///   [`ReadRangeAck::log_records`](bacnet_services::read_range::ReadRangeAck::log_records)
///   to keep them.
///
/// The standard numbers the record after the top of the range 1. A device
/// that numbers it 0 instead is one number ahead of this reader past its
/// wrap: a page that ends at or crosses the top ([`LogPage::wrapped`]) sets
/// `next` to 1, which that device gave the record after its 0. Reading on
/// loses that 0 record without a gap; resuming a caught-up log from such a
/// checkpoint can instead start over from the oldest record without
/// reporting a skip. Such devices usually also clamp a reference past the
/// wrap, which fails as [`Error::LogNotAdvancing`].
pub async fn read_log_page<R: LogRequester + ?Sized>(
    requester: &R,
    mac: &[u8],
    log: ObjectIdentifier,
    cursor: LogCursor,
    page_size: u16,
) -> Result<LogPage, Error> {
    let space = SequenceSpace::of(log.object_type())?;
    let count = page_count(page_size)?;
    let cursor = match cursor {
        LogCursor::Oldest => match window(requester, mac, log, space).await? {
            Window::Empty { next } => return Ok(empty_page(log, LogCursor::Sequence(next))),
            Window::Records { oldest, .. } => LogCursor::Sequence(oldest),
        },
        cursor => cursor,
    };
    let mut reply = requester
        .read_range_lenient(mac, &request(log, cursor, count))
        .await?;
    let LogCursor::Sequence(expected) = cursor else {
        return page(space, cursor, reply, None);
    };
    // Nothing from `asked` on: the counts tell whether the log ends there,
    // has logged it since, or no longer holds it.
    let mut asked = expected;
    let mut asked_while_held = false;
    let mut restarts = 0;
    loop {
        if reply.ack.item_count != 0 {
            return page(
                space,
                LogCursor::Sequence(asked),
                reply,
                (asked != expected).then_some(expected),
            );
        }
        if let Some(rule) = fatal(&reply.violations, LogCursor::Sequence(asked)) {
            return Err(Error::ReadRangeViolation(rule));
        }
        if restarts == RETRIES {
            break;
        }
        restarts += 1;
        asked = match window(requester, mac, log, space).await? {
            Window::Records { oldest, newest }
                if space.distance(asked, space.advance(newest, 1)) != 0 =>
            {
                let held = space.distance(oldest, asked) >= 0 && space.distance(asked, newest) >= 0;
                if held && asked_while_held {
                    break;
                }
                // Either way the record asked for next is one the counts
                // say the log holds.
                asked_while_held = true;
                if held {
                    asked
                } else {
                    oldest
                }
            }
            _ => return page(space, LogCursor::Sequence(asked), reply, None),
        };
        reply = requester
            .read_range_lenient(mac, &request(log, LogCursor::Sequence(asked), count))
            .await?;
    }
    Err(Error::LogNotAdvancing {
        requested: asked,
        returned: None,
    })
}

/// A page with no records, which reads from `next` once the log has any.
fn empty_page(log: ObjectIdentifier, next: LogCursor) -> LogPage {
    LogPage {
        records: LogRecords::empty_for(log.object_type())
            .expect("a log type was checked before reading"),
        first_sequence_number: None,
        result_flags: (false, false, false),
        gap: None,
        violations: Vec::new(),
        next,
        done: true,
        wrapped: false,
    }
}

/// The first rule in `violations` that a read from `cursor` can't
/// tolerate: an echo that doesn't match, or a page read by sequence number
/// or time without its first sequence number.
fn fatal(violations: &[ReadRangeViolation], cursor: LogCursor) -> Option<ReadRangeViolation> {
    let sequenced = !matches!(cursor, LogCursor::Position(_));
    violations
        .iter()
        .copied()
        .find(|violation| match violation {
            ReadRangeViolation::MissingFirstSequenceNumber => sequenced,
            ReadRangeViolation::ZeroFirstSequenceNumber
            | ReadRangeViolation::UnexpectedFirstSequenceNumber
            | ReadRangeViolation::MoreItemsPastEnd
            | ReadRangeViolation::ItemCountExceedsRequest => false,
            _ => true,
        })
}

/// Assemble the page `reply` answers to a read from `cursor`; `restarted`
/// is the sequence number the caller asked for when the read had to start
/// over from the oldest record.
pub(crate) fn page(
    space: SequenceSpace,
    cursor: LogCursor,
    reply: ReadRangeReply,
    restarted: Option<u64>,
) -> Result<LogPage, Error> {
    let ReadRangeReply { ack, violations } = reply;
    if let Some(rule) = fatal(&violations, cursor) {
        return Err(Error::ReadRangeViolation(rule));
    }
    let records = match ack.log_records() {
        Some(records) => records?,
        None => {
            return Err(Error::ReadRangeViolation(
                ReadRangeViolation::PropertyMismatch,
            ))
        }
    };
    let returned = u64::from(ack.item_count);
    let done = returned == 0 || ack.result_flags.1;
    let mut gap = None;
    let mut wrapped = false;
    let (first_sequence_number, next) = match (cursor, ack.first_sequence_number) {
        (LogCursor::Position(position), _) => {
            (None, LogCursor::Position(position.saturating_add(returned)))
        }
        (cursor, None) => (None, cursor),
        (cursor, Some(first)) => {
            if let LogCursor::Sequence(asked) = cursor {
                let ahead = space.distance(asked, first);
                if ahead < 0 {
                    return Err(Error::LogNotAdvancing {
                        requested: asked,
                        returned: Some(first),
                    });
                }
                let expected = restarted.unwrap_or(asked);
                let skipped = space.distance(expected, first);
                if skipped != 0 {
                    gap = Some(LogGap {
                        expected,
                        first,
                        skipped: u64::try_from(skipped).ok(),
                    });
                }
            }
            // The records run from `first` to `first + returned - 1`: they
            // reach the top of the range when that passes `max`.
            wrapped =
                first != 0 && u128::from(first) + u128::from(returned) > u128::from(space.max);
            (
                Some(first),
                LogCursor::Sequence(space.advance(first, returned)),
            )
        }
    };
    Ok(LogPage {
        records,
        first_sequence_number,
        result_flags: ack.result_flags,
        gap,
        violations,
        next,
        done,
        wrapped,
    })
}

#[cfg(test)]
#[path = "log_reader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "log_reader_device_tests.rs"]
mod device_tests;
