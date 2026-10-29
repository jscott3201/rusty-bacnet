//! Durable storage for the Trend Log, Trend Log Multiple and Event Log
//! (#1541).
//!
//! A log built with `with_persistence` (for example
//! [`TrendLogObject::with_persistence`]) reopens from the newest snapshot its
//! [`LogPersistence`] holds and then commits a fresh snapshot of itself on a
//! cadence ([`LogCommitCadence`]):
//!
//! - once Total_Record_Count has moved [`records`](LogCommitCadence::records)
//!   past the count of the last commit, as a record lands;
//! - once [`interval`](LogCommitCadence::interval) has passed since the last
//!   commit and anything the snapshot holds has changed, as the server's
//!   once-a-second operation pass looks at the log. After a quiet spell
//!   longer than the interval, the first change commits at the next pass:
//!   the interval has already run out, whichever trigger comes first;
//! - once more when the server stops, and when the log is dropped, if
//!   anything changed since the last commit.
//!
//! The defaults, 100 records or 60 seconds, bound what a power loss costs to
//! about one interval of records, while a log polling every second writes to
//! disk about once a minute instead of on every sample. A log that only an
//! application drives, with no server, commits by count and when dropped.
//!
//! The snapshot is taken under the database guard, which is only a copy of
//! the records and settings; the storage call runs on the log's own writer
//! thread ([`crate::durable`]), so no file I/O happens under the guard. A
//! commit queued while an earlier one is still waiting to start replaces it,
//! so a slow disk costs one write of the latest state.
//!
//! # Reopening
//!
//! The snapshot gives the log back its Buffer_Size, Enable, Stop_When_Full,
//! records and Total_Record_Count, its BUFFER_READY counting
//! (Last_Notify_Record and where Records_Since_Notification counts from), and
//! the settings a peer can write: Start_Time and Stop_Time, and on the trend
//! logs Logging_Type, Log_Interval, Align_Intervals, Interval_Offset and
//! Log_DeviceObjectProperty. Buffer_Size is the application's (peers can't
//! write it), so the one the constructor is given wins over the saved one:
//! a larger size keeps every record, and a smaller one keeps the newest
//! records that fit and logs a warning naming both sizes. A Buffer_Size
//! whose full buffer of the family's worst-case records couldn't fit a
//! slot's 256 MiB is refused.
//! Settings the application applies after opening replace the stored ones,
//! so an application that configures its logs on every start should skip
//! that for a log that [reopened](crate::trend::TrendLogObject::reopened).
//!
//! A reopened log then appends one LOG_INTERRUPTED status record, which
//! tells readers samples may be missing across the restart (Clauses
//! 12.25.14, 12.27.13 and 12.30.19). It is stamped with the device clock the
//! first time the log can read a valid one: as it joins an `ObjectDatabase`
//! with a clock, or at the next record or window look after that. A record
//! that arrives while no valid clock is bound lends the interruption its own
//! timestamp, so the record still lands after it. Calling
//! `record_interruption` first stands in for it. A log that reopens full
//! and stopped (Enable FALSE, as Stop_When_Full leaves it) skips the marker
//! rather than push out its oldest record; a stopped log with room takes it,
//! marked LOG_DISABLED too.
//!
//! Notification_Threshold comes back with the counts, since BUFFER_READY
//! resumes from them.
//!
//! # Failed commits
//!
//! A commit that fails is logged and leaves the log counted as changed, so
//! the next interval pass, or the final commit at stop or drop, tries again
//! even if nothing else changed.
//!
//! # Deleting a log
//!
//! Storage outlives the object: a log removed from the database leaves its
//! slots on disk, and one built again on the same storage reopens from them.
//! Delete the slot files to start afresh.
//!
//! [`TrendLogObject::with_persistence`]: crate::trend::TrendLogObject::with_persistence

use crate::prelude::*;
use core::time::Duration;

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::LoggingType;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

// The snapshot format: only the file backend reads and writes it whole.
#[cfg_attr(not(feature = "std"), allow(dead_code))]
mod codec;
#[cfg(feature = "std")]
mod file;
pub(crate) mod store;

#[cfg(feature = "std")]
pub use file::FileLogPersistence;

/// The settings that say how a Trend Log or Trend Log Multiple acquires its
/// records, as a snapshot keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogAcquisitionSettings {
    /// Logging_Type: POLLED, TRIGGERED, or on a Trend Log COV.
    pub logging_type: LoggingType,
    /// Log_Interval in hundredths of a second; zero while TRIGGERED or COV.
    pub log_interval: u32,
    /// Align_Intervals.
    pub align_intervals: bool,
    /// Interval_Offset in hundredths of a second.
    pub interval_offset: u32,
}

/// The durable state of one Trend Log (`R` = `BACnetLogRecord`), Trend Log
/// Multiple (`BACnetLogMultipleRecord`) or Event Log (`BACnetEventLogRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct LogSnapshot<R> {
    /// The log this state belongs to.
    pub object_identifier: ObjectIdentifier,
    /// Nonzero, and one higher on each commit, so a two-slot store can tell
    /// the newer slot.
    pub generation: u64,
    /// Buffer_Size.
    pub buffer_size: u32,
    /// Enable.
    pub log_enable: bool,
    /// Stop_When_Full.
    pub stop_when_full: bool,
    /// Total_Record_Count; the newest record carries this number.
    pub total_record_count: u32,
    /// The resident records, oldest first.
    pub records: Vec<R>,
    /// Last_Notify_Record.
    pub last_notify_record: u32,
    /// The Total_Record_Count Records_Since_Notification counts from: the
    /// count at the last report, purge or start of detection.
    pub records_since_notification_from: u32,
    /// Notification_Threshold: the records that make a BUFFER_READY report,
    /// zero for none.
    pub notification_threshold: u32,
    /// Start_Time, as a local date and time; every field unspecified for an
    /// open start.
    pub start_time: (Date, Time),
    /// Stop_Time, under the same rules as `start_time`.
    pub stop_time: (Date, Time),
    /// The acquisition settings of a Trend Log or Trend Log Multiple; `None`
    /// for an Event Log, which has none.
    pub acquisition: Option<LogAcquisitionSettings>,
    /// Log_DeviceObjectProperty: at most one reference on a Trend Log, up to
    /// [`MAX_LOG_DEVICE_OBJECT_PROPERTIES`](crate::trend::MAX_LOG_DEVICE_OBJECT_PROPERTIES)
    /// on a Trend Log Multiple, none on an Event Log.
    pub log_device_object_property: Vec<BACnetDeviceObjectPropertyReference>,
}

/// Application-owned storage for one log's [`LogSnapshot`].
///
/// The log calls `load` once, as it opens, and `commit` from its writer
/// thread, one call at a time.
pub trait LogPersistence<R>: Send + Sync {
    /// Load the newest valid snapshot for `expected_object`, or `None` when
    /// storage holds none yet.
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<LogSnapshot<R>>, Error>;

    /// Durably store `snapshot`, whose generation is one past the last one
    /// committed (or loaded).
    fn commit(&self, snapshot: &LogSnapshot<R>) -> Result<(), Error>;
}

/// When a durable log commits its snapshot: after `records` new records or
/// `interval` of time with a change, whichever comes first, and once more
/// as the server stops or the log is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogCommitCadence {
    /// Commit once Total_Record_Count has moved this far past the last
    /// commit. Zero turns the count trigger off.
    pub records: u32,
    /// Commit once this long has passed since the last commit and the state
    /// changed. The server looks once a second, so it is met to within about
    /// a second. Zero turns the time trigger off.
    pub interval: Duration,
}

impl LogCommitCadence {
    /// The default count trigger: 100 records.
    pub const DEFAULT_RECORDS: u32 = 100;
    /// The default time trigger: 60 seconds.
    pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
}

impl Default for LogCommitCadence {
    /// 100 records or 60 seconds.
    fn default() -> Self {
        Self {
            records: Self::DEFAULT_RECORDS,
            interval: Self::DEFAULT_INTERVAL,
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests;

#[cfg(all(test, feature = "std"))]
mod reopen_tests;

#[cfg(all(test, feature = "std"))]
mod cadence_tests;

#[cfg(all(test, feature = "std"))]
mod reopen_fit_tests;
