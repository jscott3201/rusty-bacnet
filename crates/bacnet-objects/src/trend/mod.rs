//! TrendLog (type 20) and TrendLogMultiple (type 27) objects per ASHRAE 135-2020.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetLogRecord};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LoggingType, NotifyType, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use crate::clock::ClockReader;
use crate::common::{self, read_property_list_property};
use crate::log_buffer::{
    log_buffer_read_denied, LogBufferRecords, LogRecordBuffer, LogRecordIdentity,
};
use crate::log_lifecycle::LogLifecycle;
use crate::log_reporting::{impl_buffer_ready_reporting, BufferReadyReporting};
use crate::log_window::LogWindow;
use crate::traits::BACnetObject;
use acquisition::{Acquisition, Rules};

mod acquisition;
mod metadata;
mod multiple;
mod multiple_metadata;
mod references;

pub use acquisition::DEFAULT_LOG_INTERVAL;
pub use multiple::TrendLogMultipleObject;
pub use references::MAX_LOG_DEVICE_OBJECT_PROPERTIES;

/// BACnet TrendLog object (type 20, Clause 12.25).
///
/// Ring buffer of timestamped values of the one property
/// Log_DeviceObjectProperty names. The database's trend poller samples it
/// every Log_Interval while Logging_Type is POLLED, at clock-aligned times
/// when Align_Intervals asks for them, and once for each Trigger while it is
/// TRIGGERED; the application may also call `add_record()`. Records are kept
/// only while Enable is TRUE and the local time lies between Start_Time and
/// Stop_Time. COV logging isn't carried out, so Logging_Type refuses it.
pub struct TrendLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    stop_when_full: bool,
    buffer_size: u32,
    log_buffer: LogRecordBuffer,
    reliability: Reliability,
    log_device_object_property: Option<BACnetDeviceObjectPropertyReference>,
    acquisition: Acquisition,
    window: LogWindow,
    reporting: BufferReadyReporting,
    clock: Option<Arc<dyn ClockReader>>,
}

impl TrendLogObject {
    /// Create a new Trend Log object with logging enabled; `buffer_size` is the record capacity.
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TREND_LOG, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            log_enable: true,
            stop_when_full: false,
            buffer_size,
            log_buffer: LogRecordBuffer::new(buffer_size),
            reliability: Reliability::NO_FAULT_DETECTED,
            log_device_object_property: None,
            acquisition: Acquisition::new(Rules::TrendLog),
            window: LogWindow::default(),
            reporting: BufferReadyReporting::default(),
            clock: None,
        })
    }

    /// Add a BACnetLogRecord to the trend log buffer.
    ///
    /// Success does not guarantee a resident ordinary record: disabled logging
    /// and a record outside the Start_Time / Stop_Time window are ignored,
    /// zero-capacity logging may only count, and a stop-before-full
    /// transition records status instead. Missing/invalid status clocks fail
    /// atomically with DEVICE / OPERATIONAL_PROBLEM. A successful call serves
    /// a pending Trigger, which reads FALSE again, even when the record is
    /// ignored (Enable FALSE, or outside the window): the acquisition was
    /// made.
    pub fn add_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.lifecycle().try_add_ordinary(record)?;
        self.acquisition.acquired();
        Ok(())
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetLogRecord> {
        self.log_buffer.records()
    }

    /// Total_Record_Count: the records collected since the log was created
    /// or restored, going from 2^32 - 1 on to 1. The newest record in
    /// [`records`](Self::records) carries this number. Save the two to
    /// [restore](Self::restore_log_buffer) the log later.
    pub fn total_record_count(&self) -> u32 {
        self.log_buffer.total_record_count()
    }

    /// Restore the log buffer, as a device restarting with saved records
    /// does, or seed Total_Record_Count with no records, as a test that
    /// needs the count near its wrap does (#1537).
    ///
    /// `records` become the resident records, oldest first. The newest is
    /// numbered `total_record_count` and each one before it one less, going
    /// from 1 back to 2^32 - 1, so a count below the number of records is a
    /// count that has wrapped. The records are kept as given, whatever
    /// Enable and the Start_Time / Stop_Time window say, and nothing is
    /// recorded for the restore. BUFFER_READY counts from the restored
    /// count, as when detection starts.
    ///
    /// Fails with [`Error::OutOfRange`], or a record's encoding error,
    /// leaving the log unchanged, when there are more records than
    /// Buffer_Size, when records come with a count of zero, when Stop_When_Full
    /// and Enable are both TRUE and the records fill the buffer (such a log
    /// stops before its last slot is taken), or when a record would not
    /// encode.
    ///
    /// Restore a log before adding it to an `ObjectDatabase`. A running
    /// server reaches its objects only through `BACnetObject`, which offers
    /// no restore on purpose: renumbering a log that peers are reading would
    /// change what their sequence numbers mean, with no BUFFER_PURGED record
    /// to tell them, and the BUFFER_READY reports already sent would name
    /// counts that no longer hold.
    ///
    /// A device restoring its saved records after a restart then calls
    /// [`record_interruption`](Self::record_interruption) with the time it
    /// came back: Clause 12.25.14 gives a log that status when a power
    /// failure or reset broke its collection, so readers know samples may
    /// be missing. That record counts toward Total_Record_Count like any
    /// other, numbered one past the restored count.
    pub fn restore_log_buffer(
        &mut self,
        total_record_count: u32,
        records: impl IntoIterator<Item = BACnetLogRecord>,
    ) -> Result<(), Error> {
        self.lifecycle()
            .restore(total_record_count, records.into_iter().collect())
    }

    /// Append a LOG_INTERRUPTED status record stamped `date` and `time`, as
    /// a log restored after a restart does (#1537); see
    /// [`restore_log_buffer`](Self::restore_log_buffer).
    ///
    /// The record goes in whatever Enable and the window say, counts toward
    /// Total_Record_Count, and pushes out the oldest record of a full
    /// buffer. It also carries LOG_DISABLED while collection is off; when it
    /// fills a Stop_When_Full buffer it does so and turns Enable FALSE, as
    /// the log's own status records do. Without a clock bound, as before the
    /// log is added to a database, the time has to come from the caller: a
    /// `date` and `time` that aren't an actual moment (every field given,
    /// the weekday matching the date) fail with [`Error::OutOfRange`] and
    /// change nothing.
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

    /// Set Log_DeviceObjectProperty, the property the log samples, as local
    /// configuration: the log buffer is left as it is.
    ///
    /// A reference may name another device; the poller then logs a failure
    /// for each sample instead of reading it. `None`, or a reference whose
    /// object or Device is at the reserved instance 4194303, leaves the log
    /// without one (#1417). A Device member that isn't a Device identifier
    /// fails with PROPERTY / VALUE_OUT_OF_RANGE and changes nothing.
    pub fn set_log_device_object_property(
        &mut self,
        reference: Option<BACnetDeviceObjectPropertyReference>,
    ) -> Result<(), Error> {
        if let Some(reference) = &reference {
            crate::device_reference::check_device_member(reference.device_identifier)?;
        }
        self.log_device_object_property = reference.and_then(crate::device_reference::set_or_unset);
        Ok(())
    }

    /// A client's write of Log_DeviceObjectProperty (#1234): one reference,
    /// the unset form clearing it (#1417). See [`references::check_written`]
    /// for the refusals; Null is no reference, so INVALID_DATA_TYPE. A new
    /// value purges the buffer, leaving a BUFFER_PURGED status record (Clause
    /// 12.25.8); without a valid clock the purge fails with DEVICE /
    /// OPERATIONAL_PROBLEM and nothing changes. Writing the value already held
    /// changes nothing.
    fn write_log_device_object_property(&mut self, value: PropertyValue) -> Result<(), Error> {
        let reference = crate::device_reference::decode_reference(&value)?;
        references::check_written(&reference)?;
        let reference = crate::device_reference::set_or_unset(reference);
        if reference != self.log_device_object_property {
            self.lifecycle().purge()?;
            self.log_device_object_property = reference;
        }
        Ok(())
    }

    /// Set Logging_Type, as a client's write does: POLLED or TRIGGERED. This
    /// device has no COV acquisition yet (#1480), so COV, like any other
    /// value, is PROPERTY / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (Clause
    /// 12.25.26) and changes nothing. POLLED with a zero Log_Interval sets
    /// [`DEFAULT_LOG_INTERVAL`]; TRIGGERED sets Log_Interval to zero.
    pub fn set_logging_type(&mut self, logging_type: LoggingType) -> Result<(), Error> {
        self.acquisition.set_logging_type(logging_type)
    }

    /// Set Log_Interval in hundredths of a second, as a client's write does.
    /// While Logging_Type is TRIGGERED it is read-only and this is PROPERTY /
    /// WRITE_ACCESS_DENIED. A POLLED log's nonzero interval set to zero would
    /// switch it to COV logging (Clause 12.25.9), which is refused with
    /// PROPERTY / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as COV itself is.
    pub fn set_log_interval(&mut self, hundredths: u32) -> Result<(), Error> {
        self.acquisition.set_log_interval(hundredths)
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

    /// Set Align_Intervals: whether a POLLED log acquires at clock-aligned
    /// times.
    pub fn set_align_intervals(&mut self, align: bool) {
        self.acquisition.set_align_intervals(align);
    }

    /// Set Interval_Offset, the delay after each aligned boundary, in
    /// hundredths; it applies modulo Log_Interval.
    pub fn set_interval_offset(&mut self, hundredths: u32) {
        self.acquisition.set_interval_offset(hundredths);
    }

    /// Ask for one acquisition, as a local process writing Trigger TRUE does.
    /// Only a TRIGGERED log takes it; any other is PROPERTY /
    /// NOT_CONFIGURED_FOR_TRIGGERED_LOGGING.
    pub fn trigger(&mut self) -> Result<(), Error> {
        self.acquisition.trigger()
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

    fn lifecycle(&mut self) -> LogLifecycle<'_, BACnetLogRecord> {
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

impl BACnetObject for TrendLogObject {
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
        // Logging_Type, Log_Interval, Align_Intervals, Interval_Offset and
        // Trigger, then Start_Time and Stop_Time.
        if let Some(value) = self
            .acquisition
            .read(property)
            .or_else(|| self.window.read(property))
        {
            return Ok(value);
        }
        let total = self.log_buffer.total_record_count();
        if let Some(result) = self.reporting.read(property, array_index, total) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::TREND_LOG.to_raw()))
            }
            p if p == PropertyIdentifier::LOG_ENABLE => Ok(PropertyValue::Boolean(self.log_enable)),
            p if p == PropertyIdentifier::STOP_WHEN_FULL => {
                Ok(PropertyValue::Boolean(self.stop_when_full))
            }
            p if p == PropertyIdentifier::BUFFER_SIZE => {
                Ok(PropertyValue::Unsigned(self.buffer_size as u64))
            }
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.records().len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(u64::from(total)))
            }
            // Clause 12.25.30 lets only IN_ALARM (from Event_State) and FAULT
            // (from Reliability) move on a Trend Log; OVERRIDDEN and
            // OUT_OF_SERVICE are always FALSE. Table 12-29 has no
            // Out_Of_Service property (#985).
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                StatusFlags::empty(),
                self.reliability,
                false,
                self.reporting.event_state(),
            )),
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability.to_raw()))
            }
            // ReadRange pages it through `log_buffer_internal`.
            p if p == PropertyIdentifier::LOG_BUFFER => Err(log_buffer_read_denied()),
            // The Clause 21 encoding; while no reference is set, the empty
            // element a Trend Log Multiple grows by (#1417).
            p if p == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY => {
                Ok(crate::device_reference::optional_reference_value(
                    self.log_device_object_property.as_ref(),
                    ObjectType::ANALOG_INPUT,
                ))
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
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
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if let Some(result) = self.acquisition.write(property, &value) {
            return result;
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
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            if let PropertyValue::Unsigned(0) = value {
                return self.lifecycle().purge();
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        // Clause 12.25 Table 12-29 lists Reliability as plain O with no
        // writability footnote, and unlike the intrinsic-reporting objects the
        // Trend Log Reliability_Evaluation_Inhibit paragraph requires
        // NO_FAULT_DETECTED while evaluation is inhibited, without an exception
        // for a client-supplied Reliability value while Out_Of_Service is TRUE.
        // Nothing in Clause 12.25 grants a
        // network client this property: the log owns it (logging status and
        // fault indication), so every write is refused.
        if property == PropertyIdentifier::RELIABILITY {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY {
            return self.write_log_device_object_property(value);
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

    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.add_record(record)
    }

    fn refresh_log_window_internal(&mut self) -> bool {
        self.lifecycle().refresh_window()
    }

    impl_buffer_ready_reporting!(reporting, log_buffer);
}

#[cfg(test)]
mod log_record_tests;

#[cfg(test)]
mod multiple_options_tests;

#[cfg(test)]
mod options_tests;

#[cfg(test)]
mod reference_tests;

#[cfg(test)]
mod tests;
