//! A log object's Log_Buffer as ReadRange pages it (#1092, #1237).
//!
//! Every built-in log answers ReadProperty of Log_Buffer with
//! READ_ACCESS_DENIED: Clauses 12.25.14, 12.27.13, 12.30.19 and 12.64.10 open
//! the buffer to ReadRange (and, for an Audit Log, AuditLogQuery) only.
//! ReadRange reads the records straight from the object instead. An Audit
//! Log's come from its `AuditLogStorage`, the store AuditLogQuery scans, so
//! both services see the same records, each item one bare
//! BACnetAuditLogRecord whose identity is its Unsigned64 sequence number and
//! timestamp. A Trend Log, Event Log or Trend Log Multiple lends its record
//! buffer through `log_buffer_internal`, each item one record framed as its
//! Clause 21 production (#1233), with the identities its buffer derives. By
//! Position, By Sequence Number and By Time then select through the same
//! code for every log.
//!
//! None of them builds the whole buffer's identities (#1536). A Trend Log,
//! Event Log or Trend Log Multiple computes where a sequence number sits
//! and bisects its timestamps while they run in order; an Audit Log's
//! numbers run on by one from its oldest record, so a sequence number's
//! place is computed there too, and checked. Only the page's own records
//! are then visited.

use std::collections::VecDeque;

use super::*;
use bacnet_encoding::constructed::encode_audit_log_record;
use bacnet_objects::log_buffer::{LogBufferRecords, TimestampKey, TimestampOrder};
use bacnet_types::constructed::BACnetAuditLogRecordResult;

/// The items one ReadRange page draws from.
pub(crate) enum RangeItems<'a> {
    /// One value per item, encoded with the page's value encoder.
    Values(Vec<PropertyValue>),
    /// An Audit Log's retained ring, oldest first. A page encodes only the
    /// records it visits, so a narrow window over a full log stays cheap.
    AuditRecords(&'a VecDeque<BACnetAuditLogRecordResult>),
    /// A Trend Log, Event Log or Trend Log Multiple buffer, oldest first,
    /// likewise encoded only where a page visits it.
    LogRecords(&'a dyn LogBufferRecords),
}

impl RangeItems<'_> {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Values(items) => items.len(),
            Self::AuditRecords(records) => records.len(),
            Self::LogRecords(records) => records.record_count(),
        }
    }

    /// Encode item `index`, handing values to `encode_value`.
    pub(super) fn encode_with<F>(
        &self,
        index: usize,
        buf: &mut BytesMut,
        encode_value: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(&mut BytesMut, &PropertyValue) -> Result<(), Error>,
    {
        match self {
            Self::Values(items) => encode_value(buf, &items[index]),
            Self::AuditRecords(records) => encode_audit_log_record(&records[index].record, buf),
            // A log refuses a record at admission unless it encodes.
            Self::LogRecords(records) => {
                records.encode_record(index, buf);
                Ok(())
            }
        }
    }
}

/// The identities ReadRange selects a log's records by, aligned with its
/// items.
pub(crate) enum LogIdentities<'a> {
    /// A Trend Log, Event Log or Trend Log Multiple buffer, which derives
    /// each identity from its Total_Record_Count and keeps its timestamp
    /// order up to date.
    Buffer(&'a dyn LogBufferRecords),
    /// An Audit Log's retained ring, each record carrying its own number.
    Audit(&'a VecDeque<BACnetAuditLogRecordResult>),
    /// The identities an object lists for a Log_Buffer it serves through
    /// ReadProperty.
    Listed(Vec<LogRecordIdentity>),
}

impl LogIdentities<'_> {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Buffer(records) => records.record_count(),
            Self::Audit(records) => records.len(),
            Self::Listed(identities) => identities.len(),
        }
    }

    /// The sequence number of the record at `index`.
    pub(super) fn sequence_number(&self, index: usize) -> u64 {
        match self {
            Self::Buffer(records) => records.record_identity(index).sequence_number(),
            Self::Audit(records) => records[index].sequence_number,
            Self::Listed(identities) => identities[index].sequence_number(),
        }
    }

    /// The index of the record numbered `sequence_number`, if one is
    /// resident. An Audit Log record numbered zero leaves the ring
    /// unnumbered; the store never keeps one.
    pub(super) fn position(&self, sequence_number: u64) -> Result<Option<usize>, Error> {
        match self {
            Self::Buffer(records) => Ok(records.record_position(sequence_number)),
            Self::Audit(records) => {
                // The store numbers its records on by one from the oldest,
                // wrapping 2^64 - 1 to 1, so the number says where to look.
                let computed = records.front().and_then(|oldest| {
                    let offset = sequence_offset(oldest.sequence_number, sequence_number)?;
                    usize::try_from(offset).ok()
                });
                if let Some(index) = computed.filter(|&index| {
                    records
                        .get(index)
                        .is_some_and(|record| record.sequence_number == sequence_number)
                }) {
                    return Ok(Some(index));
                }
                if records.iter().any(|record| record.sequence_number == 0) {
                    return Err(super::list_item_not_numbered());
                }
                Ok(records
                    .iter()
                    .position(|record| record.sequence_number == sequence_number))
            }
            Self::Listed(identities) => Ok(identities
                .iter()
                .position(|identity| identity.sequence_number() == sequence_number)),
        }
    }

    /// The resident index a By-Time read with `count` starts or ends at:
    /// the first record stamped after `reference` for a positive count, the
    /// last stamped before it otherwise. A timestamp that is not an actual
    /// moment anywhere in the log refuses the read, whatever it selects.
    pub(super) fn time_anchor(
        &self,
        reference: TimestampKey,
        count: i32,
    ) -> Result<Option<usize>, Error> {
        match self {
            Self::Buffer(records) => {
                let len = records.record_count();
                let key = |index| records.record_identity(index).timestamp_key();
                Ok(match records.timestamp_order() {
                    TimestampOrder::Unkeyed => return Err(super::list_item_not_timestamped()),
                    TimestampOrder::Ascending => bisect_time_anchor(len, key, reference, count),
                    // Every timestamp is keyed, so nothing is left to
                    // validate: the walk stops at the anchor.
                    TimestampOrder::Unordered if count > 0 => {
                        (0..len).find(|&index| key(index).is_some_and(|key| key > reference))
                    }
                    TimestampOrder::Unordered => (0..len)
                        .rev()
                        .find(|&index| key(index).is_some_and(|key| key < reference)),
                })
            }
            Self::Audit(records) => scan_time_anchor(
                records.len(),
                |index| {
                    let record = &records[index];
                    let (date, time) = record.record.timestamp;
                    LogRecordIdentity::new(record.sequence_number, date, time)
                        .and_then(|identity| identity.timestamp_key())
                },
                reference,
                count,
            ),
            Self::Listed(identities) => scan_time_anchor(
                identities.len(),
                |index| identities[index].timestamp_key(),
                reference,
                count,
            ),
        }
    }
}

/// How many places after `from` the number `to` comes, numbers running 1 to
/// 2^64 - 1 and then starting again at 1; `None` when either is zero.
fn sequence_offset(from: u64, to: u64) -> Option<u64> {
    if from == 0 || to == 0 {
        return None;
    }
    Some(if to >= from {
        to - from
    } else {
        // Round the wrap: to the last number, then from 1 on to `to`.
        (u64::MAX - from) + to
    })
}

/// [`LogIdentities::time_anchor`] over every record in resident order,
/// refusing the read when any timestamp is not an actual moment.
fn scan_time_anchor(
    len: usize,
    key: impl Fn(usize) -> Option<TimestampKey>,
    reference: TimestampKey,
    count: i32,
) -> Result<Option<usize>, Error> {
    let mut first_after = None;
    let mut last_before = None;
    for index in 0..len {
        let key = key(index).ok_or_else(super::list_item_not_timestamped)?;
        if first_after.is_none() && key > reference {
            first_after = Some(index);
        }
        if key < reference {
            last_before = Some(index);
        }
    }
    Ok(if count > 0 { first_after } else { last_before })
}

/// [`LogIdentities::time_anchor`] by bisection, for timestamps that never
/// go back: the records stamped at or before `reference` come first, and
/// those stamped before it are a prefix of them.
fn bisect_time_anchor(
    len: usize,
    key: impl Fn(usize) -> Option<TimestampKey>,
    reference: TimestampKey,
    count: i32,
) -> Option<usize> {
    let reference = Some(reference);
    if count > 0 {
        let after = partition_point(len, |index| key(index) <= reference);
        (after < len).then_some(after)
    } else {
        partition_point(len, |index| key(index) < reference).checked_sub(1)
    }
}

/// The first index in `0..len` where `before` stops holding, for a `before`
/// that holds on a prefix of the indices and nowhere after it.
fn partition_point(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if before(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// The resolved Log_Buffer of a log object: its items, and the identities
/// they are selected by.
pub(super) type LogBuffer<'a> = (RangeItems<'a>, LogIdentities<'a>);

/// Resolve `LOG_BUFFER` on an object that serves it outside ReadProperty (an
/// Audit Log store or a log record buffer), or `None` to leave the request to
/// the ordinary list path.
///
/// The property exists and is a BACnetLIST, so an array index fails the way
/// the ordinary path fails it on any list (Clause 15.8.1.3.1).
pub(super) fn log_buffer<'a>(
    object: &'a dyn BACnetObject,
    request: &ReadRangeRequest,
) -> Result<Option<LogBuffer<'a>>, Error> {
    if request.property_identifier != PropertyIdentifier::LOG_BUFFER {
        return Ok(None);
    }
    let resolved = if let Some(storage) = object.audit_log_storage_internal() {
        let records = storage.retained_records();
        (
            RangeItems::AuditRecords(records),
            LogIdentities::Audit(records),
        )
    } else if let Some(records) = object.log_buffer_internal() {
        (
            RangeItems::LogRecords(records),
            LogIdentities::Buffer(records),
        )
    } else {
        return Ok(None);
    };
    if request.property_array_index.is_some() {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        });
    }
    Ok(Some(resolved))
}

#[cfg(test)]
mod tests {
    use super::sequence_offset;

    #[test]
    fn audit_offsets_count_on_from_the_oldest_and_round_the_wrap() {
        assert_eq!(sequence_offset(5, 5), Some(0));
        assert_eq!(sequence_offset(5, 9), Some(4));
        assert_eq!(sequence_offset(u64::MAX - 1, u64::MAX), Some(1));
        assert_eq!(sequence_offset(u64::MAX, 1), Some(1));
        assert_eq!(sequence_offset(u64::MAX - 1, 2), Some(3));
        // An earlier number lies all the way round.
        assert_eq!(sequence_offset(9, 5), Some(u64::MAX - 4));
        assert_eq!(sequence_offset(0, 5), None);
        assert_eq!(sequence_offset(5, 0), None);
    }
}
