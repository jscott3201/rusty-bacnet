//! EventLog (type 25) object per ASHRAE 135-2020 Clause 12.27.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetEventLogRecord;
use bacnet_types::enums::{NotifyType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use crate::clock::ClockReader;
use crate::common::{self, read_common_properties};
use crate::log_buffer::{
    log_buffer_read_denied, LogBufferRecords, LogRecordBuffer, LogRecordIdentity,
};
use crate::log_lifecycle::LogLifecycle;
use crate::log_reporting::{impl_buffer_ready_reporting, BufferReadyReporting};
use crate::log_window::LogWindow;
use crate::traits::BACnetObject;

mod metadata;

/// BACnet EventLog object.
///
/// Ring buffer of timestamped event log records. A server running the
/// database logs the event notifications the device builds into its Event
/// Logs ([`ObjectDatabase::log_event_notification`]), and those it receives
/// into the logs that opt in with
/// [`set_log_received_notifications`](Self::set_log_received_notifications);
/// the application calls `add_record()` for anything else, such as a clock
/// change. The log adds its own status records. Records are kept only while
/// Enable is TRUE and the local time lies between Start_Time and Stop_Time;
/// the database's trend poller looks at that window on every pass, so an
/// opening or closing is recorded even when no record arrives.
///
/// The log reports intrinsically with the BUFFER_READY algorithm (Clause
/// 13.3.7): once Notification_Threshold is set, the server tells the
/// recipients of its Notification Class each time that many more records
/// have been collected. No log takes a record for such a report.
///
/// [`ObjectDatabase::log_event_notification`]: crate::database::ObjectDatabase::log_event_notification
pub struct EventLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    stop_when_full: bool,
    buffer_size: u32,
    log_buffer: LogRecordBuffer<BACnetEventLogRecord>,
    status_flags: StatusFlags,
    reliability: Reliability,
    window: LogWindow,
    reporting: BufferReadyReporting,
    log_received_notifications: bool,
    clock: Option<Arc<dyn ClockReader>>,
}

impl EventLogObject {
    /// Create a new EventLog object.
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::EVENT_LOG, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            log_enable: true,
            stop_when_full: false,
            buffer_size,
            log_buffer: LogRecordBuffer::new(buffer_size),
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            window: LogWindow::default(),
            reporting: BufferReadyReporting::default(),
            log_received_notifications: false,
            clock: None,
        })
    }

    /// Add a record to the event log buffer.
    ///
    /// The record is kept as given; ReadRange serves its encoding, which for
    /// an ACK_NOTIFICATION leaves out ack-required, from-state and event
    /// values (see `EventLogDatum::Notification`).
    ///
    /// Success does not guarantee a resident ordinary record: disabled logging
    /// and a record outside the Start_Time / Stop_Time window are ignored,
    /// zero-capacity logging may only count, and a stop-before-full
    /// transition records status instead. Missing/invalid status clocks fail
    /// atomically with DEVICE / OPERATIONAL_PROBLEM.
    pub fn add_record(&mut self, record: BACnetEventLogRecord) -> Result<(), Error> {
        self.lifecycle().try_add_ordinary(record).map(|_| ())
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetEventLogRecord> {
        self.log_buffer.records()
    }

    /// Total_Record_Count, as
    /// [`TrendLogObject::total_record_count`](crate::trend::TrendLogObject::total_record_count)
    /// serves it.
    pub fn total_record_count(&self) -> u32 {
        self.log_buffer.total_record_count()
    }

    /// Restore the log buffer, or seed Total_Record_Count with no records,
    /// before the log is added to an `ObjectDatabase`, under the rules of
    /// [`TrendLogObject::restore_log_buffer`](crate::trend::TrendLogObject::restore_log_buffer)
    /// (#1537). After a restart, follow it with
    /// [`record_interruption`](Self::record_interruption), which counts
    /// toward Total_Record_Count.
    pub fn restore_log_buffer(
        &mut self,
        total_record_count: u32,
        records: impl IntoIterator<Item = BACnetEventLogRecord>,
    ) -> Result<(), Error> {
        self.lifecycle()
            .restore(total_record_count, records.into_iter().collect())
    }

    /// Append a LOG_INTERRUPTED status record, as
    /// [`TrendLogObject::record_interruption`](crate::trend::TrendLogObject::record_interruption)
    /// does, after restoring the log on a restart (#1537).
    pub fn record_interruption(&mut self, date: Date, time: Time) -> Result<(), Error> {
        self.lifecycle().record_interruption((date, time))
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.log_buffer.clear();
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set Start_Time, the local date and time from which records are kept,
    /// as local configuration: nothing is recorded for the change itself,
    /// but the log notes at once where the window stands, so a client's
    /// write that then opens or shuts it is recorded. Every field
    /// unspecified leaves the start open. Any other value has to name an
    /// actual date and time, or it is PROPERTY / VALUE_OUT_OF_RANGE: the
    /// weekday may stay unspecified, and unspecified seconds or hundredths
    /// count as zero.
    pub fn set_start_time(&mut self, date: Date, time: Time) -> Result<(), Error> {
        self.lifecycle()
            .configure_window(PropertyIdentifier::START_TIME, (date, time))
    }

    /// Set Stop_Time, the local date and time from which records are no
    /// longer kept, under the same rules as
    /// [`set_start_time`](Self::set_start_time).
    pub fn set_stop_time(&mut self, date: Date, time: Time) -> Result<(), Error> {
        self.lifecycle()
            .configure_window(PropertyIdentifier::STOP_TIME, (date, time))
    }

    /// Whether the log takes the event notifications the device receives,
    /// as well as those it builds. Off unless set.
    pub fn logs_received_notifications(&self) -> bool {
        self.log_received_notifications
    }

    /// Have the log take the Confirmed and UnconfirmedEventNotifications the
    /// device receives, unicast or broadcast, each kept as it decoded, or
    /// stop it doing so. Clause 12.27 leaves the choice to the device, and
    /// it's off by default. A running server holds what it logs to a small
    /// number of records per source each second; see
    /// [`ObjectDatabase::log_received_event_notification`](crate::database::ObjectDatabase::log_received_event_notification).
    pub fn set_log_received_notifications(&mut self, log: bool) {
        self.log_received_notifications = log;
    }

    /// Set Notification_Threshold, the number of records that makes a
    /// BUFFER_READY report; zero, the default, makes none.
    pub fn set_notification_threshold(&mut self, threshold: u32) {
        self.reporting.set_notification_threshold(threshold);
    }

    /// Set Notification_Class: the number of the Notification Class whose
    /// recipients get the log's reports (0 by default).
    pub fn set_notification_class(&mut self, class: u32) {
        self.reporting.set_notification_class(class);
    }

    /// Set Event_Enable; a report goes out only while its TO_NORMAL flag is
    /// set, as it is by default.
    pub fn set_event_enable(&mut self, enable: EventTransitionBits) {
        self.reporting.set_event_enable(enable);
    }

    /// Set Notify_Type, sent with each report (EVENT by default).
    pub fn set_notify_type(&mut self, notify_type: NotifyType) {
        self.reporting.set_notify_type(notify_type);
    }

    fn lifecycle(&mut self) -> LogLifecycle<'_, BACnetEventLogRecord> {
        LogLifecycle::new(
            &mut self.log_buffer,
            &mut self.log_enable,
            &mut self.stop_when_full,
            self.clock.as_ref(),
            &mut self.window,
            &mut self.reporting,
        )
    }
}

impl BACnetObject for EventLogObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-31 has neither Out_Of_Service nor Log_Interval (#1064), and
        // Clause 12.27 holds the OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        if let Some(value) = self.window.read(property) {
            return Ok(value);
        }
        let total = self.log_buffer.total_record_count();
        if let Some(result) = self.reporting.read(property, array_index, total) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::EVENT_LOG.to_raw()))
            }
            p if p == PropertyIdentifier::LOG_ENABLE => Ok(PropertyValue::Boolean(self.log_enable)),
            p if p == PropertyIdentifier::STOP_WHEN_FULL => {
                Ok(PropertyValue::Boolean(self.stop_when_full))
            }
            p if p == PropertyIdentifier::BUFFER_SIZE => {
                Ok(PropertyValue::Unsigned(self.buffer_size as u64))
            }
            // ReadRange pages it through `log_buffer_internal`.
            p if p == PropertyIdentifier::LOG_BUFFER => Err(log_buffer_read_denied()),
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.records().len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(u64::from(total)))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::LOG_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                return self.lifecycle().write_enable(v);
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = self.window.write(property, &value) {
            // A change that opens or closes the window is recorded at once.
            result?;
            self.lifecycle().refresh_window();
            return Ok(());
        }
        if property == PropertyIdentifier::STOP_WHEN_FULL {
            if let PropertyValue::Boolean(v) = value {
                return self.lifecycle().write_stop_when_full(v);
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            if let PropertyValue::Unsigned(0) = value {
                return self.lifecycle().purge();
            }
            return Err(common::invalid_data_type_error());
        }
        let total = self.log_buffer.total_record_count();
        if let Some(result) = self.reporting.write(property, array_index, &value, total) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        Some(self.log_buffer.identities())
    }

    fn log_buffer_internal(&self) -> Option<&dyn LogBufferRecords> {
        Some(&self.log_buffer)
    }

    fn add_event_log_record(&mut self, record: BACnetEventLogRecord) -> Result<(), Error> {
        self.add_record(record)
    }

    fn refresh_log_window_internal(&mut self) -> bool {
        self.lifecycle().refresh_window()
    }

    fn logs_received_event_notifications_internal(&self) -> bool {
        self.log_received_notifications
    }

    impl_buffer_ready_reporting!(reporting, log_buffer);
}

#[cfg(test)]
mod tests;
