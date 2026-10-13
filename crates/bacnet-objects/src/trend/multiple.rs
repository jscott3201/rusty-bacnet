//! TrendLogMultiple (type 27, Clause 12.30).

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetLogMultipleRecord};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LoggingType, NotifyType, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use super::acquisition::{Acquisition, Rules};
use super::multiple_metadata;
use super::references::{self, MAX_LOG_DEVICE_OBJECT_PROPERTIES};
use crate::clock::ClockReader;
use crate::common::{self, read_property_list_property};
use crate::device_reference;
use crate::log_buffer::{
    log_buffer_read_denied, LogBufferRecords, LogRecordBuffer, LogRecordIdentity,
};
use crate::log_lifecycle::LogLifecycle;
use crate::log_reporting::{impl_buffer_ready_reporting, BufferReadyReporting};
use crate::log_window::LogWindow;
use crate::traits::BACnetObject;

/// BACnet TrendLogMultiple object (type 27).
///
/// Multi-channel trending. Unlike TrendLog, which monitors a single property,
/// TrendLogMultiple monitors a list of device-object-property references and
/// logs one value per reference in each record. The database's trend poller
/// samples every reference every Log_Interval while Logging_Type is POLLED,
/// at clock-aligned times when Align_Intervals asks for them, and once for
/// each Trigger while it is TRIGGERED. Records are kept only while Enable is
/// TRUE and the local time lies between Start_Time and Stop_Time.
pub struct TrendLogMultipleObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    stop_when_full: bool,
    buffer_size: u32,
    log_buffer: LogRecordBuffer<BACnetLogMultipleRecord>,
    log_device_object_property: Vec<BACnetDeviceObjectPropertyReference>,
    acquisition: Acquisition,
    window: LogWindow,
    reporting: BufferReadyReporting,
    reliability: Reliability,
    clock: Option<Arc<dyn ClockReader>>,
}

impl TrendLogMultipleObject {
    /// Build a log with logging enabled that holds at most `buffer_size`
    /// records.
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            log_enable: true,
            stop_when_full: false,
            buffer_size,
            log_buffer: LogRecordBuffer::new(buffer_size),
            log_device_object_property: Vec::new(),
            acquisition: Acquisition::new(Rules::TrendLogMultiple),
            window: LogWindow::default(),
            reporting: BufferReadyReporting::default(),
            reliability: Reliability::NO_FAULT_DETECTED,
            clock: None,
        })
    }

    /// Add a record to the log buffer.
    ///
    /// Success does not guarantee a resident ordinary record: disabled logging
    /// and a record outside the Start_Time / Stop_Time window are ignored,
    /// zero-capacity logging may only count, and a stop-before-full
    /// transition records status instead. Missing/invalid status clocks fail
    /// atomically with DEVICE / OPERATIONAL_PROBLEM. A successful call serves
    /// a pending Trigger, which reads FALSE again, even when the record is
    /// ignored (Enable FALSE, or outside the window): the acquisition was
    /// made.
    pub fn add_record(&mut self, record: BACnetLogMultipleRecord) -> Result<(), Error> {
        self.lifecycle().try_add_ordinary(record)?;
        self.acquisition.acquired();
        Ok(())
    }

    /// Append a reference to Log_DeviceObjectProperty as local configuration:
    /// the log buffer is left as it is.
    ///
    /// A reference may name another device; the poller then logs a failure
    /// in its slot of each record instead of reading it. A Device member that
    /// isn't a Device identifier fails with PROPERTY / VALUE_OUT_OF_RANGE, and
    /// a reference past [`MAX_LOG_DEVICE_OBJECT_PROPERTIES`] with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY; either leaves the array unchanged.
    pub fn add_property_reference(
        &mut self,
        reference: BACnetDeviceObjectPropertyReference,
    ) -> Result<(), Error> {
        device_reference::check_device_member(reference.device_identifier)?;
        if self.log_device_object_property.len() >= MAX_LOG_DEVICE_OBJECT_PROPERTIES {
            return Err(references::no_space_error());
        }
        self.log_device_object_property.push(reference);
        Ok(())
    }

    /// A client's write of Log_DeviceObjectProperty (#1234).
    ///
    /// A whole write replaces the array, at any length up to
    /// [`MAX_LOG_DEVICE_OBJECT_PROPERTIES`]; an indexed write replaces one
    /// element. Writing index 0 resizes the array (Clause 12.1.5.1): a smaller
    /// Unsigned drops the trailing elements, a larger one appends empty
    /// elements, one past the cap is RESOURCES / NO_SPACE_TO_WRITE_PROPERTY and
    /// another datatype PROPERTY / INVALID_DATA_TYPE. Each written reference
    /// goes through [`references::check_written`], where an element naming
    /// instance 4194303 is an empty one. A new value purges the buffer,
    /// leaving a BUFFER_PURGED status record, which is the first of the two
    /// actions Clause 12.30.11 offers; without a valid clock the purge fails
    /// with DEVICE / OPERATIONAL_PROBLEM and nothing changes. Writing the value
    /// already held changes nothing.
    fn write_log_device_object_property(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let mut candidate = self.log_device_object_property.clone();
        match array_index {
            None => {
                let written = device_reference::decode_references(&value)?;
                if written.len() > MAX_LOG_DEVICE_OBJECT_PROPERTIES {
                    return Err(references::no_space_error());
                }
                for reference in &written {
                    references::check_written(reference)?;
                }
                candidate = written;
            }
            Some(0) => {
                let PropertyValue::Unsigned(size) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let size = usize::try_from(size)
                    .ok()
                    .filter(|size| *size <= MAX_LOG_DEVICE_OBJECT_PROPERTIES)
                    .ok_or_else(references::no_space_error)?;
                candidate.resize_with(size, references::empty_element);
            }
            Some(index) => {
                let slot = usize::try_from(index - 1)
                    .ok()
                    .and_then(|slot| candidate.get_mut(slot))
                    .ok_or_else(common::invalid_array_index_error)?;
                let reference = device_reference::decode_reference(&value)?;
                references::check_written(&reference)?;
                *slot = reference;
            }
        }
        if candidate != self.log_device_object_property {
            self.lifecycle().purge()?;
            self.log_device_object_property = candidate;
        }
        Ok(())
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetLogMultipleRecord> {
        self.log_buffer.records()
    }

    /// Total_Record_Count, as
    /// [`TrendLogObject::total_record_count`](super::TrendLogObject::total_record_count)
    /// serves it.
    pub fn total_record_count(&self) -> u32 {
        self.log_buffer.total_record_count()
    }

    /// Restore the log buffer, or seed Total_Record_Count with no records,
    /// before the log is added to an `ObjectDatabase`, under the rules of
    /// [`TrendLogObject::restore_log_buffer`](super::TrendLogObject::restore_log_buffer)
    /// (#1537). Records aren't checked against Log_DeviceObjectProperty,
    /// which may have changed since they were taken. After a restart, follow
    /// it with [`record_interruption`](Self::record_interruption), which
    /// counts toward Total_Record_Count.
    pub fn restore_log_buffer(
        &mut self,
        total_record_count: u32,
        records: impl IntoIterator<Item = BACnetLogMultipleRecord>,
    ) -> Result<(), Error> {
        self.lifecycle()
            .restore(total_record_count, records.into_iter().collect())
    }

    /// Append a LOG_INTERRUPTED status record, as
    /// [`TrendLogObject::record_interruption`](super::TrendLogObject::record_interruption)
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

    /// Set Logging_Type, as a client's write does: POLLED or TRIGGERED. COV
    /// logging isn't allowed for this object type (Clause 12.30.12), so COV,
    /// like any other value, is PROPERTY / VALUE_OUT_OF_RANGE and changes
    /// nothing. POLLED with a zero Log_Interval sets
    /// [`DEFAULT_LOG_INTERVAL`](super::DEFAULT_LOG_INTERVAL); TRIGGERED sets
    /// Log_Interval to zero.
    pub fn set_logging_type(&mut self, logging_type: LoggingType) -> Result<(), Error> {
        self.acquisition.set_logging_type(logging_type)
    }

    /// Set Log_Interval in hundredths of a second. While Logging_Type is
    /// TRIGGERED it is read-only and this is PROPERTY / WRITE_ACCESS_DENIED.
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

    pub(super) fn logging_type(&self) -> LoggingType {
        self.acquisition.logging_type()
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

    fn lifecycle(&mut self) -> LogLifecycle<'_, BACnetLogMultipleRecord> {
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

impl BACnetObject for TrendLogMultipleObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::TREND_LOG_MULTIPLE.to_raw(),
            )),
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
            // Clause 12.30.5 lets only IN_ALARM (from Event_State) and FAULT
            // (from Reliability) move on a Trend Log Multiple; OVERRIDDEN and
            // OUT_OF_SERVICE are always FALSE. Table 12-35 has no
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
            // A BACnetARRAY: one Clause 21 encoding per element (#1234).
            p if p == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY => common::read_array(
                device_reference::reference_elements(&self.log_device_object_property),
                array_index,
            ),
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
        if property == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY {
            return self.write_log_device_object_property(array_index, value);
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
        multiple_metadata::for_object(self)
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

    fn add_trend_multiple_record(&mut self, record: BACnetLogMultipleRecord) -> Result<(), Error> {
        self.add_record(record)
    }

    fn refresh_log_window_internal(&mut self) -> bool {
        self.lifecycle().refresh_window()
    }

    impl_buffer_ready_reporting!(reporting, log_buffer);
}
