//! Shared resident storage and identity projection for BACnet log objects.

use std::collections::VecDeque;

use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::primitives::{Date, PropertyValue, Time};

/// Stable object-owned identity for one resident log record.
///
/// Identity views returned by [`crate::traits::BACnetObject::log_record_identities_internal`]
/// are ordered oldest-to-newest and align element-for-element with the owning
/// object's resident records and `LOG_BUFFER` projection. Sequence numbers are
/// always nonzero; after `u32::MAX`, the next accepted record uses 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRecordIdentity {
    sequence_number: u32,
    date: Date,
    time: Time,
}

impl LogRecordIdentity {
    /// Construct an identity, rejecting zero as an invalid record sequence.
    pub fn new(sequence_number: u32, date: Date, time: Time) -> Option<Self> {
        (sequence_number != 0).then_some(Self {
            sequence_number,
            date,
            time,
        })
    }

    /// Return this record's nonzero Unsigned32 sequence number.
    pub fn sequence_number(&self) -> u32 {
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
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LogRecordProfile {
    Event,
    Trend,
    TrendMultiple,
}

pub(crate) struct LogRecordBufferRecords(VecDeque<BACnetLogRecord>);

pub(crate) struct LogRecordBuffer {
    capacity: u32,
    records: VecDeque<BACnetLogRecord>,
    total_record_count: u32,
}

impl LogRecordBuffer {
    pub(crate) fn new(capacity: u32) -> Self {
        Self {
            capacity,
            records: VecDeque::new(),
            total_record_count: 0,
        }
    }

    pub(crate) fn append(
        &mut self,
        record: BACnetLogRecord,
        enabled: bool,
        stop_when_full: bool,
    ) -> bool {
        let full = self.records.len() >= self.capacity as usize;
        if !enabled || (full && stop_when_full) {
            return false;
        }

        let sequence_number = next_sequence(self.total_record_count);
        self.total_record_count = sequence_number;
        if full {
            self.records.pop_front();
        }
        self.records.push_back(record);
        true
    }

    pub(crate) fn records(&self) -> &VecDeque<BACnetLogRecord> {
        &self.records
    }

    pub(crate) fn total_record_count(&self) -> u32 {
        self.total_record_count
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }

    pub(crate) fn identities(&self) -> Vec<LogRecordIdentity> {
        if self.records.is_empty() {
            return Vec::new();
        }

        debug_assert_ne!(self.total_record_count, 0);
        let mut sequence_number = self.total_record_count;
        for _ in 1..self.records.len() {
            sequence_number = previous_sequence(sequence_number);
        }

        self.records
            .iter()
            .map(|record| {
                let identity = LogRecordIdentity {
                    sequence_number,
                    date: record.date,
                    time: record.time,
                };
                sequence_number = next_sequence(sequence_number);
                identity
            })
            .collect()
    }

    pub(crate) fn project(&self, profile: LogRecordProfile) -> PropertyValue {
        PropertyValue::List(
            self.records
                .iter()
                .map(|record| project_record(record, profile))
                .collect(),
        )
    }

    pub(crate) fn take_records(&mut self) -> LogRecordBufferRecords {
        LogRecordBufferRecords(std::mem::take(&mut self.records))
    }

    pub(crate) fn restore_records(&mut self, records: LogRecordBufferRecords) {
        debug_assert!(self.records.is_empty());
        self.records = records.0;
    }

    #[cfg(test)]
    pub(crate) fn set_total_record_count_for_test(&mut self, total_record_count: u32) {
        debug_assert!(self.records.is_empty());
        self.total_record_count = total_record_count;
    }
}

fn next_sequence(sequence_number: u32) -> u32 {
    if sequence_number == u32::MAX {
        1
    } else {
        sequence_number + 1
    }
}

fn previous_sequence(sequence_number: u32) -> u32 {
    if sequence_number == 1 {
        u32::MAX
    } else {
        sequence_number - 1
    }
}

fn project_record(record: &BACnetLogRecord, profile: LogRecordProfile) -> PropertyValue {
    let mut fields = vec![
        PropertyValue::Date(record.date),
        PropertyValue::Time(record.time),
        project_datum(&record.log_datum),
    ];
    if let (LogRecordProfile::Trend, Some(status_flags)) = (profile, record.status_flags) {
        fields.push(PropertyValue::BitString {
            unused_bits: 4,
            data: vec![status_flags << 4],
        });
    }
    PropertyValue::List(fields)
}

fn project_datum(datum: &LogDatum) -> PropertyValue {
    match datum {
        LogDatum::LogStatus(value) => PropertyValue::Unsigned(*value as u64),
        LogDatum::BooleanValue(value) => PropertyValue::Boolean(*value),
        LogDatum::RealValue(value) => PropertyValue::Real(*value),
        LogDatum::EnumValue(value) => PropertyValue::Enumerated(*value),
        LogDatum::UnsignedValue(value) => PropertyValue::Unsigned(*value),
        LogDatum::SignedValue(value) => PropertyValue::Signed(*value as i32),
        LogDatum::BitstringValue { unused_bits, data } => PropertyValue::BitString {
            unused_bits: *unused_bits,
            data: data.clone(),
        },
        LogDatum::NullValue => PropertyValue::Null,
        LogDatum::Failure {
            error_class,
            error_code,
        } => PropertyValue::List(vec![
            PropertyValue::Unsigned(*error_class as u64),
            PropertyValue::Unsigned(*error_code as u64),
        ]),
        LogDatum::TimeChange(value) => PropertyValue::Real(*value),
        LogDatum::AnyValue(bytes) => PropertyValue::OctetString(bytes.clone()),
    }
}

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

        assert!(buffer.append(record(1), true, false));
        assert_eq!(buffer.identities()[0].sequence_number(), 1);

        buffer.clear();
        buffer.set_total_record_count_for_test(u32::MAX);
        assert!(buffer.append(record(2), true, false));
        assert_eq!(buffer.total_record_count(), 1);
        assert_eq!(buffer.identities()[0].sequence_number(), 1);
    }

    #[test]
    fn log_buffer_rejections_do_not_consume_sequence() {
        let mut buffer = LogRecordBuffer::new(1);
        assert!(!buffer.append(record(1), false, false));
        assert_eq!(buffer.total_record_count(), 0);

        assert!(buffer.append(record(2), true, true));
        assert!(!buffer.append(record(3), true, true));
        assert_eq!(buffer.total_record_count(), 1);
        assert_eq!(buffer.identities()[0].sequence_number(), 1);
    }

    #[test]
    fn log_buffer_preserves_zero_capacity_runtime_behavior() {
        let mut ring = LogRecordBuffer::new(0);
        assert!(ring.append(record(1), true, false));
        assert!(ring.append(record(2), true, false));
        assert_eq!(ring.records().len(), 1);
        assert_eq!(ring.records()[0].time.hour, 2);
        assert_eq!(ring.total_record_count(), 2);

        let mut stop_when_full = LogRecordBuffer::new(0);
        assert!(!stop_when_full.append(record(1), true, true));
        assert!(stop_when_full.records().is_empty());
        assert_eq!(stop_when_full.total_record_count(), 0);
    }

    #[test]
    fn log_buffer_fifo_eviction_keeps_survivor_identity_and_alignment() {
        let mut buffer = LogRecordBuffer::new(2);
        assert!(buffer.append(record(1), true, false));
        assert!(buffer.append(record(2), true, false));
        let survivor = buffer.identities()[1];
        assert!(buffer.append(record(3), true, false));

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
        buffer.set_total_record_count_for_test(u32::MAX - 1);
        assert!(buffer.append(record(1), true, false));
        assert!(buffer.append(record(2), true, false));
        assert!(buffer.append(record(3), true, false));
        assert_eq!(
            buffer
                .identities()
                .iter()
                .map(LogRecordIdentity::sequence_number)
                .collect::<Vec<_>>(),
            vec![u32::MAX, 1, 2]
        );

        assert!(buffer.append(record(4), true, false));
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
        assert!(buffer.append(record(1), true, false));
        assert!(buffer.append(record(2), true, false));
        buffer.clear();

        assert!(buffer.records().is_empty());
        assert!(buffer.identities().is_empty());
        assert_eq!(buffer.total_record_count(), 2);
        assert!(buffer.append(record(3), true, false));
        assert_eq!(buffer.identities()[0].sequence_number(), 3);
        assert_eq!(LogRecordBuffer::new(2).total_record_count(), 0);
    }

    #[test]
    fn log_buffer_duplicate_timestamps_keep_distinct_sequences() {
        let mut buffer = LogRecordBuffer::new(2);
        let duplicate = record(4);
        assert!(buffer.append(duplicate.clone(), true, false));
        assert!(buffer.append(duplicate, true, false));

        let identities = buffer.identities();
        assert_eq!(identities[0].date(), identities[1].date());
        assert_eq!(identities[0].time(), identities[1].time());
        assert_eq!(identities[0].sequence_number(), 1);
        assert_eq!(identities[1].sequence_number(), 2);
    }

    #[test]
    fn log_buffer_move_restore_preserves_payloads_and_derived_identities() {
        let mut buffer = LogRecordBuffer::new(2);
        assert!(buffer.append(record(1), true, false));
        assert!(buffer.append(record(2), true, false));
        let records = buffer.records().clone();
        let identities = buffer.identities();
        let total = buffer.total_record_count();

        let moved = buffer.take_records();
        assert!(buffer.records().is_empty());
        assert_eq!(buffer.total_record_count(), total);
        buffer.restore_records(moved);

        assert_eq!(buffer.records(), &records);
        assert_eq!(buffer.identities(), identities);
        assert_eq!(buffer.total_record_count(), total);
    }
}
