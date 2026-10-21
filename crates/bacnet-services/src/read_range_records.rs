//! Typed log records from a ReadRange acknowledgement's item data (#1534).
//!
//! A Trend Log, Event Log, Trend Log Multiple or Audit Log answers a ReadRange
//! of its Log_Buffer with records laid back to back in the item data, each in
//! its own record production (Clauses 12.25.14, 12.27.13, 12.30.19 and
//! 12.64.10). These decoders walk that list, so a caller never loops the
//! record codecs by hand.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use core::fmt;

use bacnet_encoding::constructed::{
    decode_audit_log_record_at, decode_event_log_record, decode_log_multiple_record,
    decode_log_record,
};
use bacnet_types::constructed::{
    BACnetAuditLogRecord, BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;

use super::ReadRangeAck;

/// A record kind a log object keeps in its Log_Buffer, decodable one after
/// another from ReadRange item data.
pub trait LogBufferRecord: Sized {
    /// Decode one record starting at `offset`, returning it and the offset
    /// just past it.
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error>;
}

impl LogBufferRecord for BACnetLogRecord {
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_log_record(data, offset)
    }
}

impl LogBufferRecord for BACnetEventLogRecord {
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_event_log_record(data, offset)
    }
}

impl LogBufferRecord for BACnetLogMultipleRecord {
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_log_multiple_record(data, offset)
    }
}

impl LogBufferRecord for BACnetAuditLogRecord {
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_audit_log_record_at(data, offset)
    }
}

/// The records of one page, typed by the kind of log that holds them.
#[derive(Debug, Clone, PartialEq)]
pub enum LogRecords {
    /// Trend Log records.
    TrendLog(Vec<BACnetLogRecord>),
    /// Event Log records.
    EventLog(Vec<BACnetEventLogRecord>),
    /// Trend Log Multiple records.
    TrendLogMultiple(Vec<BACnetLogMultipleRecord>),
    /// Audit Log records.
    AuditLog(Vec<BACnetAuditLogRecord>),
}

impl LogRecords {
    /// An empty list of the record kind `object_type` keeps, or `None` when
    /// it is not a log type.
    pub fn empty_for(object_type: ObjectType) -> Option<Self> {
        Some(match object_type {
            ObjectType::TREND_LOG => Self::TrendLog(Vec::new()),
            ObjectType::EVENT_LOG => Self::EventLog(Vec::new()),
            ObjectType::TREND_LOG_MULTIPLE => Self::TrendLogMultiple(Vec::new()),
            ObjectType::AUDIT_LOG => Self::AuditLog(Vec::new()),
            _ => return None,
        })
    }

    /// How many records there are.
    pub fn len(&self) -> usize {
        match self {
            Self::TrendLog(records) => records.len(),
            Self::EventLog(records) => records.len(),
            Self::TrendLogMultiple(records) => records.len(),
            Self::AuditLog(records) => records.len(),
        }
    }

    /// Whether there are no records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Item data that didn't decode as the records the acknowledgement claims.
///
/// `decoded` keeps the records before the one that failed, so a caller can
/// still use them; `offset` is where that record starts in the item data.
#[derive(Debug)]
pub struct LogRecordsError<C> {
    /// The records that decoded, in order, before the failure.
    pub decoded: C,
    /// Zero-based index of the record that failed.
    pub index: usize,
    /// Offset in the item data where the record that failed starts; the item
    /// data's length when records are missing at the end.
    pub offset: usize,
    /// Why that record failed.
    pub error: Error,
}

impl<C> LogRecordsError<C> {
    fn map<D>(self, f: impl FnOnce(C) -> D) -> LogRecordsError<D> {
        LogRecordsError {
            decoded: f(self.decoded),
            index: self.index,
            offset: self.offset,
            error: self.error,
        }
    }
}

impl<C> fmt::Display for LogRecordsError<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "log record {} at item-data offset {}: {}",
            self.index, self.offset, self.error
        )
    }
}

impl<C: fmt::Debug> core::error::Error for LogRecordsError<C> {}

impl<C> From<LogRecordsError<C>> for Error {
    /// A [`Error::Decoding`] at the failing record's offset, naming the
    /// record by its index and keeping the inner fault's kind and label
    /// (see [`Error::to_decode_error`]).
    fn from(error: LogRecordsError<C>) -> Self {
        let refused = error.error.to_decode_error(error.offset).within("log record");
        match u16::try_from(error.index) {
            Ok(index) => refused.at_item(index).into(),
            Err(_) => refused.into(),
        }
    }
}

impl ReadRangeAck {
    /// Decode the item data as records of kind `R`.
    ///
    /// Every octet must belong to a record, and the number of records must
    /// equal [`item_count`](Self::item_count). It doesn't check that the
    /// acknowledgement is for a log of `R`'s type;
    /// [`log_records`](Self::log_records) picks the kind from the object
    /// identifier.
    pub fn records<R: LogBufferRecord>(&self) -> Result<Vec<R>, LogRecordsError<Vec<R>>> {
        let data = self.item_data.as_slice();
        let expected = usize::try_from(self.item_count).unwrap_or(usize::MAX);
        // Item count comes off the wire: never reserve more than the data can
        // hold, at two octets or more per record.
        let mut records = Vec::with_capacity(expected.min(data.len() / 2));
        let mut offset = 0;
        let fault = loop {
            if offset == data.len() {
                if records.len() == expected {
                    return Ok(records);
                }
                break Error::missing(offset, "item data holds fewer records than item-count");
            }
            if records.len() == expected {
                break Error::trailing(offset, "item data holds more records than item-count");
            }
            match R::decode_at(data, offset) {
                Ok((record, next)) if next > offset => {
                    records.push(record);
                    offset = next;
                }
                Ok(_) => break Error::decoding(offset, "log record has no octets"),
                Err(error) => break error,
            }
        };
        Err(LogRecordsError {
            index: records.len(),
            decoded: records,
            offset,
            error: fault,
        })
    }

    /// Decode the item data as Trend Log records.
    pub fn trend_log_records(
        &self,
    ) -> Result<Vec<BACnetLogRecord>, LogRecordsError<Vec<BACnetLogRecord>>> {
        self.records()
    }

    /// Decode the item data as Event Log records.
    pub fn event_log_records(
        &self,
    ) -> Result<Vec<BACnetEventLogRecord>, LogRecordsError<Vec<BACnetEventLogRecord>>> {
        self.records()
    }

    /// Decode the item data as Trend Log Multiple records.
    pub fn trend_log_multiple_records(
        &self,
    ) -> Result<Vec<BACnetLogMultipleRecord>, LogRecordsError<Vec<BACnetLogMultipleRecord>>> {
        self.records()
    }

    /// Decode the item data as Audit Log records.
    pub fn audit_log_records(
        &self,
    ) -> Result<Vec<BACnetAuditLogRecord>, LogRecordsError<Vec<BACnetAuditLogRecord>>> {
        self.records()
    }

    /// Decode the item data as the records of the log that answered: `None`
    /// unless the acknowledgement is for the Log_Buffer of a Trend Log, Event
    /// Log, Trend Log Multiple or Audit Log.
    pub fn log_records(&self) -> Option<Result<LogRecords, LogRecordsError<LogRecords>>> {
        if self.property_identifier != PropertyIdentifier::LOG_BUFFER {
            return None;
        }
        Some(match self.object_identifier.object_type() {
            ObjectType::TREND_LOG => self
                .records()
                .map(LogRecords::TrendLog)
                .map_err(|e| e.map(LogRecords::TrendLog)),
            ObjectType::EVENT_LOG => self
                .records()
                .map(LogRecords::EventLog)
                .map_err(|e| e.map(LogRecords::EventLog)),
            ObjectType::TREND_LOG_MULTIPLE => self
                .records()
                .map(LogRecords::TrendLogMultiple)
                .map_err(|e| e.map(LogRecords::TrendLogMultiple)),
            ObjectType::AUDIT_LOG => self
                .records()
                .map(LogRecords::AuditLog)
                .map_err(|e| e.map(LogRecords::AuditLog)),
            _ => return None,
        })
    }
}

#[cfg(test)]
#[path = "read_range_records_tests.rs"]
mod tests;
