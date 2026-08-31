//! EventLog (type 25) object per ASHRAE 135-2020 Clause 12.28.

use std::borrow::Cow;
use std::collections::VecDeque;

use bacnet_types::constructed::BACnetLogRecord;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::log_buffer::{
    LogRecordBuffer, LogRecordBufferRecords, LogRecordIdentity, LogRecordProfile,
};
use crate::traits::{BACnetObject, WritePropertyRollback};

struct EventLogWriteRollback {
    records: LogRecordBufferRecords,
}

/// BACnet EventLog object.
///
/// Ring buffer of timestamped event log records. The application calls
/// `add_record()` to log event data. Resident records retain the legacy shared
/// Rust `BACnetLogRecord` shape; Event Log projection remains family-specific.
pub struct EventLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    log_interval: u32,
    stop_when_full: bool,
    buffer_size: u32,
    log_buffer: LogRecordBuffer,
    status_flags: StatusFlags,
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
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
            log_interval: 0,
            stop_when_full: false,
            buffer_size,
            log_buffer: LogRecordBuffer::new(buffer_size),
            status_flags: StatusFlags::empty(),
            event_state: 0,
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a BACnetLogRecord to the event log buffer.
    pub fn add_record(&mut self, record: BACnetLogRecord) {
        self.log_buffer
            .append(record, self.log_enable, self.stop_when_full);
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetLogRecord> {
        self.log_buffer.records()
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.log_buffer.clear();
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::EVENT_LOG.to_raw()))
            }
            p if p == PropertyIdentifier::LOG_ENABLE => Ok(PropertyValue::Boolean(self.log_enable)),
            p if p == PropertyIdentifier::LOG_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.log_interval as u64))
            }
            p if p == PropertyIdentifier::STOP_WHEN_FULL => {
                Ok(PropertyValue::Boolean(self.stop_when_full))
            }
            p if p == PropertyIdentifier::BUFFER_SIZE => {
                Ok(PropertyValue::Unsigned(self.buffer_size as u64))
            }
            p if p == PropertyIdentifier::LOG_BUFFER => {
                Ok(self.log_buffer.project(LogRecordProfile::Event))
            }
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.records().len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => Ok(PropertyValue::Unsigned(
                self.log_buffer.total_record_count() as u64,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::LOG_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.log_enable = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::LOG_INTERVAL {
            if let PropertyValue::Unsigned(v) = value {
                self.log_interval = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::STOP_WHEN_FULL {
            if let PropertyValue::Boolean(v) = value {
                self.stop_when_full = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            // Writing 0 clears the buffer
            if let PropertyValue::Unsigned(0) = value {
                self.clear();
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::LOG_ENABLE,
            PropertyIdentifier::LOG_INTERVAL,
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyIdentifier::LOG_BUFFER,
            PropertyIdentifier::RECORD_COUNT,
            PropertyIdentifier::TOTAL_RECORD_COUNT,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::RECORD_COUNT
            && matches!(value, PropertyValue::Unsigned(0)))
        .then(|| {
            WritePropertyRollback::new(EventLogWriteRollback {
                records: self.log_buffer.take_records(),
            })
        })
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        self.log_buffer
            .restore_records(rollback.downcast::<EventLogWriteRollback>()?.records);
        Ok(())
    }

    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        Some(self.log_buffer.identities())
    }
}

#[cfg(test)]
mod tests;
