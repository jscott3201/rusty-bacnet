//! The log-buffer capability: a Trend Log, Trend Log Multiple or Event Log's
//! resident records as ReadRange pages them, the records the host hands it,
//! and its Start_Time / Stop_Time window.

use crate::prelude::*;

use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventNotificationRequest,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::not_supported;
use crate::database::ObjectDatabase;
use crate::log_buffer::{LogBufferRecords, LogRecordIdentity};

/// The step that records an event notification this device built in its
/// Event Logs, `logs`: what
/// [`ObjectDatabase::log_event_notification`] runs once a log lends its
/// buffer. It comes from the log ([`LogBuffer::notification_logger`]), so
/// an image whose objects lend no log buffer links none of it (#2524).
#[doc(hidden)]
pub type NotificationLogger =
    fn(&mut ObjectDatabase, Vec<ObjectIdentifier>, &EventNotificationRequest);

/// A log object whose buffer the host pages and fills.
///
/// The object returns itself from
/// [`BACnetObject::log_buffer`](crate::traits::BACnetObject::log_buffer),
/// for the ReadRange reads the bundled server makes under a shared database
/// guard, and from
/// [`log_buffer_mut`](crate::traits::BACnetObject::log_buffer_mut) for the
/// records the trend poller and the event logger add under the write guard.
/// An object that lends one lends the other. The built-in Trend Log, Trend
/// Log Multiple and Event Log objects lend it; on one of those, call its own
/// `add_record` instead. Not public API yet (#2179).
#[doc(hidden)]
pub trait LogBuffer {
    /// Stable identities aligned with the resident records, oldest to
    /// newest, in the same order as the log's Log_Buffer. ReadRange selects
    /// records by sequence number and time through them. Sequence numbers
    /// are object-owned metadata and never part of an encoded BACnet record.
    fn record_identities(&self) -> Vec<LogRecordIdentity>;

    /// The resident Log_Buffer records as ReadRange pages them.
    ///
    /// The built-in logs answer a ReadProperty of Log_Buffer with PROPERTY /
    /// READ_ACCESS_DENIED, since their clauses (12.25.14, 12.30.19, 12.27.13)
    /// open the buffer to ReadRange only; the server's ReadRange handler
    /// reads it here instead, one encoded record per item. The default has
    /// none, which leaves ReadRange reading Log_Buffer through
    /// [`read_property`](crate::traits::BACnetObject::read_property).
    fn buffer_records(&self) -> Option<&dyn LogBufferRecords> {
        None
    }

    /// Submit a trend record to the log's lifecycle.
    ///
    /// `Ok(())` means the operation was accepted, not that an ordinary
    /// record became resident: a disabled log ignores it and a zero-capacity
    /// log may only increment its count. A required status transition can
    /// replace the ordinary record. A timestamp failure returns an error
    /// without mutation. The default, for a log that takes no trend records,
    /// answers OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn add_trend_record(&mut self, _record: BACnetLogRecord) -> Result<(), Error> {
        Err(not_supported())
    }

    /// Submit a Trend Log Multiple record, one value per monitored member.
    /// The outcomes and the default match
    /// [`add_trend_record`](Self::add_trend_record).
    fn add_trend_multiple_record(&mut self, _record: BACnetLogMultipleRecord) -> Result<(), Error> {
        Err(not_supported())
    }

    /// Submit an Event Log record: the bundled server hands each event
    /// notification this device builds to its Event Logs through this
    /// ([`ObjectDatabase::log_event_notification`](crate::database::ObjectDatabase::log_event_notification)).
    /// The outcomes and the default match
    /// [`add_trend_record`](Self::add_trend_record).
    fn add_event_log_record(&mut self, _record: BACnetEventLogRecord) -> Result<(), Error> {
        Err(not_supported())
    }

    /// Look at the Start_Time / Stop_Time window against the bound clock,
    /// recording the LOG_DISABLED change when it opened or closed since the
    /// last look while Enable is TRUE.
    ///
    /// The trend poller calls this for every log on each pass, so a window
    /// that moves while no record arrives is still logged within one pass.
    /// Returns whether a record was added. The default, for a log without a
    /// window, does nothing and returns `false`.
    fn refresh_window(&mut self) -> bool {
        false
    }

    /// The database's step that records the device's own event
    /// notifications in its Event Logs. It is reached through a log's
    /// vtable, not called directly, so that only an image holding a log
    /// object links it. Keep the default.
    #[doc(hidden)]
    fn notification_logger(&self) -> NotificationLogger {
        ObjectDatabase::record_event_notification
    }
}

/// The two accessors of an object that lends itself as its [`LogBuffer`],
/// for inside its `BACnetObject` impl.
macro_rules! lend_log_buffer {
    () => {
        fn log_buffer(&self) -> Option<&dyn $crate::traits::LogBuffer> {
            Some(self)
        }

        fn log_buffer_mut(&mut self) -> Option<&mut dyn $crate::traits::LogBuffer> {
            Some(self)
        }
    };
}

/// [`LogBuffer`] for a built-in log: its `log_buffer` field pages the
/// records, its `add_record` takes the `$add` hook's records, and its
/// lifecycle refreshes the window.
macro_rules! impl_log_buffer {
    ($object:ty, $add:ident, $record:ty) => {
        impl $crate::traits::LogBuffer for $object {
            fn record_identities(&self) -> Vec<$crate::log_buffer::LogRecordIdentity> {
                self.log_buffer.identities()
            }

            fn buffer_records(&self) -> Option<&dyn $crate::log_buffer::LogBufferRecords> {
                Some(&self.log_buffer)
            }

            fn $add(&mut self, record: $record) -> Result<(), bacnet_types::error::Error> {
                self.add_record(record)
            }

            fn refresh_window(&mut self) -> bool {
                self.settle_interruption(None);
                let recorded = self.lifecycle().refresh_window();
                self.commit_if_counted();
                recorded
            }
        }
    };
}

pub(crate) use {impl_log_buffer, lend_log_buffer};
