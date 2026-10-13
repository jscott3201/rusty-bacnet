//! Shared resident storage and identity projection for BACnet log objects.

use std::collections::VecDeque;

use bacnet_encoding::constructed::{
    encode_event_log_record, encode_log_multiple_record, encode_log_record,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

use crate::property_metadata::{
    PropertyConformance::RequiredRead, PropertyMetadata, PropertyWriteCapability::ReadOnly,
};

// Shared conformance rows only. LOG_BUFFER is a present, read-only BACnetLIST
// that ReadProperty refuses; ReadRange pages it through `LogBufferRecords`.
pub(crate) const BUFFER_SIZE_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::BUFFER_SIZE, RequiredRead, None, ReadOnly);
pub(crate) const LOG_BUFFER_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::LOG_BUFFER, RequiredRead, None, ReadOnly);
pub(crate) const TOTAL_RECORD_COUNT_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::TOTAL_RECORD_COUNT, RequiredRead, None, ReadOnly);

/// The answer a log object gives a ReadProperty of its Log_Buffer.
///
/// Clauses 12.25.14, 12.27.13, 12.30.19 and 12.64.10 open a log buffer to
/// ReadRange (and, for an Audit Log, AuditLogQuery) only, so a property read
/// names the property as present but not readable this way
/// (Clause 15.5.1.3.1).
pub(crate) fn log_buffer_read_denied() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::READ_ACCESS_DENIED.to_raw() as u32,
    }
}

/// A log object's Log_Buffer as ReadRange pages it: the resident records,
/// oldest first, each framed on demand as its Clause 21 record production,
/// with the identity ReadRange selects it by.
///
/// The records align element for element with
/// [`crate::traits::BACnetObject::log_record_identities_internal`]. A page
/// encodes only the records it visits, and the built-in logs find them
/// without walking the whole buffer, so a narrow window over a full log
/// stays cheap (#1536).
pub trait LogBufferRecords {
    /// The number of resident records.
    fn record_count(&self) -> usize;

    /// Append the record at `index` (0 is the oldest) to `buf`.
    ///
    /// Encoding cannot fail: the built-in logs refuse a record that would not
    /// encode when it is added, and an implementation must keep that promise
    /// too. Panics when `index` is not below
    /// [`record_count`](Self::record_count).
    fn encode_record(&self, index: usize, buf: &mut BytesMut);

    /// The identity of the record at `index` (0 is the oldest). Panics when
    /// `index` is not below [`record_count`](Self::record_count).
    fn record_identity(&self, index: usize) -> LogRecordIdentity;

    /// The index of the resident record numbered `sequence_number`, or
    /// `None` when no resident record has that number.
    ///
    /// The default visits every record. The built-in logs number their
    /// records consecutively and compute the index instead.
    fn record_position(&self, sequence_number: u64) -> Option<usize> {
        (0..self.record_count())
            .find(|&index| self.record_identity(index).sequence_number() == sequence_number)
    }

    /// Whether every resident timestamp is an actual moment, and if so
    /// whether they run in order, which lets a search by time bisect them.
    ///
    /// The default visits every record. The built-in logs keep the answer
    /// up to date as records come and go.
    fn timestamp_order(&self) -> TimestampOrder {
        let mut order = TimestampOrder::Ascending;
        let mut previous = None;
        for index in 0..self.record_count() {
            let Some(key) = self.record_identity(index).timestamp_key() else {
                return TimestampOrder::Unkeyed;
            };
            if previous.is_some_and(|previous| key < previous) {
                order = TimestampOrder::Unordered;
            }
            previous = Some(key);
        }
        order
    }
}

/// A record timestamp as ReadRange By-Time compares it: the civil date and
/// time, most significant field first, with the day of the week left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimestampKey {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    hundredths: u8,
}

impl TimestampKey {
    /// The key of `date` and `time`, or `None` unless every field is
    /// specified and in range. The day of the week has to name a weekday
    /// too, though the key then leaves it out; the day of the month is
    /// checked against 31, not against its month.
    pub fn of(date: Date, time: Time) -> Option<Self> {
        let year = date.actual_year()?;
        let valid = (1..=12).contains(&date.month)
            && (1..=31).contains(&date.day)
            && (1..=7).contains(&date.day_of_week)
            && time.hour <= 23
            && time.minute <= 59
            && time.second <= 59
            && time.hundredths <= 99;
        valid.then_some(Self {
            year,
            month: date.month,
            day: date.day,
            hour: time.hour,
            minute: time.minute,
            second: time.second,
            hundredths: time.hundredths,
        })
    }
}

/// How a log buffer's timestamps stand for a search by time
/// ([`LogBufferRecords::timestamp_order`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampOrder {
    /// Some resident timestamp is not an actual moment
    /// ([`TimestampKey::of`] refuses it), so the buffer can't be searched by
    /// time at all.
    Unkeyed,
    /// Every timestamp is an actual moment and none is earlier than the one
    /// before it, so a search can bisect them.
    Ascending,
    /// Every timestamp is an actual moment, but the clock went back at least
    /// once, so a search has to visit each record.
    Unordered,
}

/// The largest encoded value, in octets, that the trend pollers log as an
/// any-value. A larger value (a long string, a big array) is logged as a
/// PROPERTY / VALUE_TOO_LONG failure instead, so that one Trend Log record
/// stays small enough for a ReadRange page on a 480-octet APDU.
pub const ANY_VALUE_MAX_OCTETS: usize = 256;

/// Stable object-owned identity for one resident log record.
///
/// Identity views returned by [`crate::traits::BACnetObject::log_record_identities_internal`]
/// are ordered oldest-to-newest and align element-for-element with the owning
/// object's resident records and `LOG_BUFFER` projection. Sequence numbers are
/// always nonzero. Event and Trend logs count in Unsigned32 and wrap from
/// `u32::MAX` to 1; Audit Log counts in Unsigned64 and wraps from `u64::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRecordIdentity {
    sequence_number: u64,
    date: Date,
    time: Time,
}

impl LogRecordIdentity {
    /// Construct an identity, rejecting zero as an invalid record sequence.
    pub fn new(sequence_number: u64, date: Date, time: Time) -> Option<Self> {
        (sequence_number != 0).then_some(Self {
            sequence_number,
            date,
            time,
        })
    }

    /// Return this record's nonzero sequence number.
    pub fn sequence_number(&self) -> u64 {
        self.sequence_number
    }

    /// Return the date owned by this record.
    pub fn date(&self) -> Date {
        self.date
    }

    /// Return the time owned by this record.
    pub fn time(&self) -> Time {
        self.time
    }

    /// This record's timestamp as ReadRange By-Time compares it, or `None`
    /// when it is not an actual moment.
    pub fn timestamp_key(&self) -> Option<TimestampKey> {
        TimestampKey::of(self.date, self.time)
    }
}

/// A record a [`LogRecordBuffer`] can hold: it carries its own timestamp,
/// the shared lifecycle can build the log-status record of its family, and
/// it encodes as its family's Clause 21 production.
pub(crate) trait ResidentLogRecord: Clone {
    /// The local date and time the record was acquired.
    fn timestamp(&self) -> (Date, Time);
    /// A log-status record carrying `status`.
    fn log_status(date: Date, time: Time, status: LogStatus) -> Self;
    /// Append the record's wire form to `buf`, leaving it unchanged on error.
    fn encode(&self, buf: &mut BytesMut) -> Result<(), Error>;
}

impl ResidentLogRecord for BACnetLogRecord {
    fn timestamp(&self) -> (Date, Time) {
        (self.date, self.time)
    }

    fn log_status(date: Date, time: Time, status: LogStatus) -> Self {
        Self {
            date,
            time,
            log_datum: LogDatum::LogStatus(status),
            status_flags: None,
        }
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        encode_log_record(self, buf)
    }
}

impl ResidentLogRecord for BACnetEventLogRecord {
    fn timestamp(&self) -> (Date, Time) {
        (self.date, self.time)
    }

    fn log_status(date: Date, time: Time, status: LogStatus) -> Self {
        Self {
            date,
            time,
            log_datum: EventLogDatum::LogStatus(status),
        }
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        encode_event_log_record(self, buf)
    }
}

impl ResidentLogRecord for BACnetLogMultipleRecord {
    fn timestamp(&self) -> (Date, Time) {
        (self.date, self.time)
    }

    fn log_status(date: Date, time: Time, status: LogStatus) -> Self {
        Self {
            date,
            time,
            log_data: LogData::LogStatus(status),
        }
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        encode_log_multiple_record(self, buf)
    }
}

/// A log's resident records and Total_Record_Count.
///
/// The records are numbered consecutively up to the count, so the buffer
/// derives any record's identity, or the index of any number, from the
/// count alone. It also keeps two tallies as records come and go, which
/// answer [`LogBufferRecords::timestamp_order`] without a walk: the
/// records whose timestamp is not an actual moment, and the neighbouring
/// pairs where the later record is stamped earlier, as after the clock was
/// set back.
#[derive(Clone)]
pub(crate) struct LogRecordBuffer<R = BACnetLogRecord> {
    capacity: u32,
    records: VecDeque<R>,
    total_record_count: u32,
    unkeyed: usize,
    descents: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrdinaryAdmission {
    IgnoredDisabled,
    Inserted,
    CountOnly,
    StopBeforeFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForcedAdmission {
    Inserted,
    CountOnly,
}

impl<R: ResidentLogRecord> LogRecordBuffer<R> {
    pub(crate) fn new(capacity: u32) -> Self {
        Self {
            capacity,
            records: VecDeque::new(),
            total_record_count: 0,
            unkeyed: 0,
            descents: 0,
        }
    }

    pub(crate) fn admit_ordinary(
        &mut self,
        record: R,
        enabled: bool,
        stop_when_full: bool,
    ) -> OrdinaryAdmission {
        if !enabled {
            return OrdinaryAdmission::IgnoredDisabled;
        }
        if stop_when_full && self.next_record_would_fill() {
            return OrdinaryAdmission::StopBeforeFull;
        }

        match self.insert_counted(record) {
            ForcedAdmission::Inserted => OrdinaryAdmission::Inserted,
            ForcedAdmission::CountOnly => OrdinaryAdmission::CountOnly,
        }
    }

    pub(crate) fn insert_forced(&mut self, record: R) -> ForcedAdmission {
        self.insert_counted(record)
    }

    pub(crate) fn is_full(&self) -> bool {
        self.records.len() >= self.capacity as usize
    }

    pub(crate) fn next_record_would_fill_positive_capacity(&self) -> bool {
        self.capacity > 0 && self.next_record_would_fill()
    }

    pub(crate) fn records(&self) -> &VecDeque<R> {
        &self.records
    }

    pub(crate) fn total_record_count(&self) -> u32 {
        self.total_record_count
    }

    pub(crate) fn capacity(&self) -> u32 {
        self.capacity
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.unkeyed = 0;
        self.descents = 0;
    }

    pub(crate) fn identities(&self) -> Vec<LogRecordIdentity> {
        (0..self.records.len())
            .map(|index| self.record_identity(index))
            .collect()
    }

    /// Replace the resident records with `records`, oldest first, the newest
    /// numbered `total_record_count` and each one before it one less, below
    /// 1 wrapping to 2^32 - 1 (#1537). A count below the number of records
    /// is therefore a count that has wrapped: Clause 12.25.16 lets the count
    /// start again at 1, after which it no longer says how many records
    /// were collected.
    ///
    /// Refused, leaving the buffer as it was: more records than Buffer_Size;
    /// records with a count of zero, which only a log that never collected
    /// one has; records that fill the buffer of a log that `stops_when_full`
    /// (Stop_When_Full and Enable both TRUE), since such a log stops before
    /// its last slot is taken; and a record that would not encode.
    pub(crate) fn restore(
        &mut self,
        total_record_count: u32,
        records: VecDeque<R>,
        stops_when_full: bool,
    ) -> Result<(), Error> {
        let held = records.len();
        if held > self.capacity as usize {
            return Err(Error::OutOfRange(format!(
                "{held} log records exceed Buffer_Size {}",
                self.capacity
            )));
        }
        if total_record_count == 0 && held > 0 {
            return Err(Error::OutOfRange(format!(
                "{held} log records restored with a Total_Record_Count of 0"
            )));
        }
        if stops_when_full && held > 0 && held >= self.capacity as usize {
            return Err(Error::OutOfRange(
                "a log with Stop_When_Full and Enable TRUE can't hold a full buffer".into(),
            ));
        }
        let mut scratch = BytesMut::new();
        for record in &records {
            record.encode(&mut scratch)?;
            scratch.clear();
        }

        self.clear();
        for record in records {
            self.push_back(record);
        }
        self.total_record_count = total_record_count;
        Ok(())
    }

    fn next_record_would_fill(&self) -> bool {
        self.capacity == 0 || self.records.len().saturating_add(1) >= self.capacity as usize
    }

    fn insert_counted(&mut self, record: R) -> ForcedAdmission {
        self.total_record_count = next_sequence(self.total_record_count);
        if self.capacity == 0 {
            return ForcedAdmission::CountOnly;
        }
        if self.is_full() {
            self.pop_front();
        }
        self.push_back(record);
        ForcedAdmission::Inserted
    }

    /// Append `record`, counting it into the timestamp tallies.
    fn push_back(&mut self, record: R) {
        if self
            .records
            .back()
            .is_some_and(|last| descends(last, &record))
        {
            self.descents += 1;
        }
        if timestamp_key(&record).is_none() {
            self.unkeyed += 1;
        }
        self.records.push_back(record);
    }

    /// Drop the oldest record, taking it out of the timestamp tallies. The
    /// pair it formed with its successor was counted when that one came.
    fn pop_front(&mut self) {
        let Some(oldest) = self.records.pop_front() else {
            return;
        };
        if timestamp_key(&oldest).is_none() {
            self.unkeyed -= 1;
        }
        if self
            .records
            .front()
            .is_some_and(|next| descends(&oldest, next))
        {
            self.descents -= 1;
        }
    }
}

impl<R: ResidentLogRecord> LogBufferRecords for LogRecordBuffer<R> {
    fn record_count(&self) -> usize {
        self.records.len()
    }

    fn encode_record(&self, index: usize, buf: &mut BytesMut) {
        // `LogLifecycle::try_add_ordinary` and `restore` refuse a record that
        // would not encode, the lifecycle's own status records always
        // encode, and a resident record is never changed.
        self.records[index]
            .encode(buf)
            .expect("every resident log record encodes");
    }

    fn record_identity(&self, index: usize) -> LogRecordIdentity {
        let (date, time) = self.records[index].timestamp();
        let newer = self.records.len() - 1 - index;
        LogRecordIdentity {
            sequence_number: u64::from(sequence_before(self.total_record_count, newer)),
            date,
            time,
        }
    }

    fn record_position(&self, sequence_number: u64) -> Option<usize> {
        let sequence_number = u32::try_from(sequence_number)
            .ok()
            .filter(|&number| number != 0)?;
        let newer = sequence_distance(sequence_number, self.total_record_count);
        let newer = usize::try_from(newer)
            .ok()
            .filter(|&newer| newer < self.records.len())?;
        Some(self.records.len() - 1 - newer)
    }

    fn timestamp_order(&self) -> TimestampOrder {
        if self.unkeyed > 0 {
            TimestampOrder::Unkeyed
        } else if self.descents > 0 {
            TimestampOrder::Unordered
        } else {
            TimestampOrder::Ascending
        }
    }
}

fn timestamp_key<R: ResidentLogRecord>(record: &R) -> Option<TimestampKey> {
    let (date, time) = record.timestamp();
    TimestampKey::of(date, time)
}

/// Whether `later`, coming right after `earlier`, is stamped before it. A
/// pair with a timestamp that is not an actual moment never counts: such a
/// record is tallied on its own, and the buffer can't be searched by time
/// while it is resident.
fn descends<R: ResidentLogRecord>(earlier: &R, later: &R) -> bool {
    matches!(
        (timestamp_key(earlier), timestamp_key(later)),
        (Some(earlier), Some(later)) if later < earlier
    )
}

/// Sequence numbers run 1 to 2^32 - 1 and then start again at 1, a cycle of
/// 2^32 - 1 numbers that never includes 0.
const SEQUENCE_CYCLE: u64 = u32::MAX as u64;

fn next_sequence(sequence_number: u32) -> u32 {
    if sequence_number == u32::MAX {
        1
    } else {
        sequence_number + 1
    }
}

/// The number `steps` places before `sequence_number` in the cycle.
fn sequence_before(sequence_number: u32, steps: usize) -> u32 {
    let steps = steps as u64 % SEQUENCE_CYCLE;
    let offset = u64::from(sequence_number).saturating_sub(1);
    ((offset + SEQUENCE_CYCLE - steps) % SEQUENCE_CYCLE) as u32 + 1
}

/// How many places after `from` the number `to` comes in the cycle.
fn sequence_distance(from: u32, to: u32) -> u64 {
    (u64::from(to) + SEQUENCE_CYCLE - u64::from(from)) % SEQUENCE_CYCLE
}

#[cfg(test)]
#[path = "log_buffer_lookup_tests.rs"]
mod lookup_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
    use bacnet_types::primitives::{Date, Time};

    fn record(hour: u8) -> BACnetLogRecord {
        BACnetLogRecord {
            date: Date {
                year: 126,
                month: 8,
                day: 31,
                day_of_week: 1,
            },
            time: Time {
                hour,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::UnsignedValue(hour as u64),
            status_flags: None,
        }
    }

    #[test]
    fn log_buffer_assigns_one_and_wraps_max_without_zero() {
        let mut buffer = LogRecordBuffer::new(2);
        assert_eq!(buffer.total_record_count(), 0);
        assert!(LogRecordIdentity::new(0, record(0).date, record(0).time).is_none());

        assert_eq!(
            buffer.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(buffer.identities()[0].sequence_number(), 1);

        buffer.clear();
        buffer.restore(u32::MAX, VecDeque::new(), false).unwrap();
        assert_eq!(
            buffer.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(buffer.total_record_count(), 1);
        assert_eq!(buffer.identities()[0].sequence_number(), 1);
    }

    #[test]
    fn log_buffer_rejections_do_not_consume_sequence() {
        let mut buffer = LogRecordBuffer::new(1);
        assert_eq!(
            buffer.admit_ordinary(record(1), false, false),
            OrdinaryAdmission::IgnoredDisabled
        );
        assert_eq!(buffer.total_record_count(), 0);

        assert_eq!(
            buffer.admit_ordinary(record(2), true, true),
            OrdinaryAdmission::StopBeforeFull
        );
        assert_eq!(buffer.total_record_count(), 0);
        assert!(buffer.identities().is_empty());
    }

    #[test]
    fn log_buffer_zero_capacity_counts_without_retaining() {
        let mut ring = LogRecordBuffer::new(0);
        assert_eq!(
            ring.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::CountOnly
        );
        assert_eq!(
            ring.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::CountOnly
        );
        assert!(ring.records().is_empty());
        assert_eq!(ring.total_record_count(), 2);

        let mut stop_when_full = LogRecordBuffer::new(0);
        assert_eq!(
            stop_when_full.admit_ordinary(record(1), true, true),
            OrdinaryAdmission::StopBeforeFull
        );
        assert!(stop_when_full.records().is_empty());
        assert_eq!(stop_when_full.total_record_count(), 0);
    }

    #[test]
    fn log_buffer_fifo_eviction_keeps_survivor_identity_and_alignment() {
        let mut buffer = LogRecordBuffer::new(2);
        assert_eq!(
            buffer.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::Inserted
        );
        let survivor = buffer.identities()[1];
        assert_eq!(
            buffer.admit_ordinary(record(3), true, false),
            OrdinaryAdmission::Inserted
        );

        let identities = buffer.identities();
        assert_eq!(
            identities,
            vec![
                survivor,
                LogRecordIdentity::new(3, record(3).date, record(3).time).unwrap()
            ]
        );
        assert_eq!(identities[0].sequence_number(), 2);
        assert_ne!(identities[0].sequence_number(), 1);
        for (record, identity) in buffer.records().iter().zip(&identities) {
            assert_eq!(identity.date(), record.date);
            assert_eq!(identity.time(), record.time);
        }
    }

    #[test]
    fn log_buffer_wrap_and_eviction_keep_modular_fifo_alignment() {
        let mut buffer = LogRecordBuffer::new(3);
        buffer
            .restore(u32::MAX - 1, VecDeque::new(), false)
            .unwrap();
        assert_eq!(
            buffer.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(record(3), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer
                .identities()
                .iter()
                .map(LogRecordIdentity::sequence_number)
                .collect::<Vec<_>>(),
            vec![u64::from(u32::MAX), 1, 2]
        );

        assert_eq!(
            buffer.admit_ordinary(record(4), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer
                .identities()
                .iter()
                .map(LogRecordIdentity::sequence_number)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            buffer
                .records()
                .iter()
                .map(|record| record.time.hour)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn log_buffer_clear_preserves_counter_and_constructor_resets_it() {
        let mut buffer = LogRecordBuffer::new(2);
        assert_eq!(
            buffer.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::Inserted
        );
        buffer.clear();

        assert!(buffer.records().is_empty());
        assert!(buffer.identities().is_empty());
        assert_eq!(buffer.total_record_count(), 2);
        assert_eq!(
            buffer.admit_ordinary(record(3), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(buffer.identities()[0].sequence_number(), 3);
        assert_eq!(
            LogRecordBuffer::<BACnetLogRecord>::new(2).total_record_count(),
            0
        );
    }

    #[test]
    fn log_buffer_duplicate_timestamps_keep_distinct_sequences() {
        let mut buffer = LogRecordBuffer::new(2);
        let duplicate = record(4);
        assert_eq!(
            buffer.admit_ordinary(duplicate.clone(), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(duplicate, true, false),
            OrdinaryAdmission::Inserted
        );

        let identities = buffer.identities();
        assert_eq!(identities[0].date(), identities[1].date());
        assert_eq!(identities[0].time(), identities[1].time());
        assert_eq!(identities[0].sequence_number(), 1);
        assert_eq!(identities[1].sequence_number(), 2);
    }

    #[test]
    fn log_buffer_clone_restore_preserves_payloads_and_derived_identities() {
        let mut buffer = LogRecordBuffer::new(2);
        assert_eq!(
            buffer.admit_ordinary(record(1), true, false),
            OrdinaryAdmission::Inserted
        );
        assert_eq!(
            buffer.admit_ordinary(record(2), true, false),
            OrdinaryAdmission::Inserted
        );
        let records = buffer.records().clone();
        let identities = buffer.identities();
        let total = buffer.total_record_count();

        let snapshot = buffer.clone();
        buffer.clear();
        buffer = snapshot;

        assert_eq!(buffer.records(), &records);
        assert_eq!(buffer.identities(), identities);
        assert_eq!(buffer.total_record_count(), total);
    }
}
