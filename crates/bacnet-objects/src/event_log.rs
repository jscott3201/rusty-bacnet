//! EventLog (type 25) object per ASHRAE 135-2020 Clause 12.28.

use std::borrow::Cow;
use std::collections::VecDeque;

use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet EventLog object.
///
/// Ring buffer of timestamped event log records. The application calls
/// `add_record()` to log event data. Uses the same `BACnetLogRecord`
/// encoding as TrendLog.
pub struct EventLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    log_interval: u32,
    stop_when_full: bool,
    buffer_size: u32,
    buffer: VecDeque<BACnetLogRecord>,
    total_record_count: u64,
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
            buffer: VecDeque::new(),
            total_record_count: 0,
            status_flags: StatusFlags::empty(),
            event_state: 0,
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a BACnetLogRecord to the event log buffer.
    pub fn add_record(&mut self, record: BACnetLogRecord) {
        if !self.log_enable {
            return;
        }
        if self.buffer.len() >= self.buffer_size as usize {
            if self.stop_when_full {
                return;
            }
            self.buffer.pop_front();
        }
        self.buffer.push_back(record);
        self.total_record_count += 1;
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetLogRecord> {
        &self.buffer
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn encode_log_buffer(&self) -> PropertyValue {
        let records = self
            .buffer
            .iter()
            .map(|record| {
                let datum_value = match &record.log_datum {
                    LogDatum::LogStatus(v) => PropertyValue::Unsigned(*v as u64),
                    LogDatum::BooleanValue(v) => PropertyValue::Boolean(*v),
                    LogDatum::RealValue(v) => PropertyValue::Real(*v),
                    LogDatum::EnumValue(v) => PropertyValue::Enumerated(*v),
                    LogDatum::UnsignedValue(v) => PropertyValue::Unsigned(*v),
                    LogDatum::SignedValue(v) => PropertyValue::Signed(*v as i32),
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
                    LogDatum::TimeChange(v) => PropertyValue::Real(*v),
                    LogDatum::AnyValue(bytes) => PropertyValue::OctetString(bytes.clone()),
                };
                PropertyValue::List(vec![
                    PropertyValue::Date(record.date),
                    PropertyValue::Time(record.time),
                    datum_value,
                ])
            })
            .collect();
        PropertyValue::List(records)
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
            p if p == PropertyIdentifier::LOG_BUFFER => Ok(self.encode_log_buffer()),
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.buffer.len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.total_record_count))
            }
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
                self.buffer.clear();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::LogDatum;
    use bacnet_types::primitives::{Date, Time};

    fn make_date() -> Date {
        Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        }
    }

    fn make_time(hour: u8) -> Time {
        Time {
            hour,
            minute: 0,
            second: 0,
            hundredths: 0,
        }
    }

    fn make_record(hour: u8, value: f32) -> BACnetLogRecord {
        BACnetLogRecord {
            date: make_date(),
            time: make_time(hour),
            log_datum: LogDatum::RealValue(value),
            status_flags: None,
        }
    }

    #[test]
    fn create_event_log() {
        let el = EventLogObject::new(1, "EL-1", 100).unwrap();
        assert_eq!(el.object_identifier().object_type(), ObjectType::EVENT_LOG);
        assert_eq!(el.object_identifier().instance_number(), 1);
        assert_eq!(el.object_name(), "EL-1");
    }

    #[test]
    fn read_object_type() {
        let el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let val = el
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::EVENT_LOG.to_raw())
        );
    }

    #[test]
    fn add_records_and_read_count() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        el.add_record(make_record(10, 72.5));
        el.add_record(make_record(11, 73.0));
        assert_eq!(el.records().len(), 2);
        let val = el
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(2));
        let val = el
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(2));
    }

    #[test]
    fn read_log_buffer() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        el.add_record(make_record(10, 72.5));
        el.add_record(make_record(11, 73.0));
        let val = el
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        if let PropertyValue::List(records) = val {
            assert_eq!(records.len(), 2);
            if let PropertyValue::List(fields) = &records[0] {
                assert_eq!(fields.len(), 3);
                assert_eq!(fields[2], PropertyValue::Real(72.5));
            } else {
                panic!("Expected List for log record");
            }
            if let PropertyValue::List(fields) = &records[1] {
                assert_eq!(fields[2], PropertyValue::Real(73.0));
            } else {
                panic!("Expected List for log record");
            }
        } else {
            panic!("Expected List for LOG_BUFFER");
        }
    }

    #[test]
    fn read_log_buffer_empty() {
        let el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let val = el
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn ring_buffer_wraps() {
        let mut el = EventLogObject::new(1, "EL-1", 3).unwrap();
        for i in 0..5u8 {
            el.add_record(BACnetLogRecord {
                date: make_date(),
                time: make_time(i),
                log_datum: LogDatum::UnsignedValue(i as u64),
                status_flags: None,
            });
        }
        assert_eq!(el.records().len(), 3);
        // Oldest records evicted; first remaining is hour=2
        assert_eq!(el.records()[0].time.hour, 2);
        let val = el
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(5));
    }

    #[test]
    fn stop_when_full() {
        let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
        el.write_property(
            PropertyIdentifier::STOP_WHEN_FULL,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        for i in 0..5u8 {
            el.add_record(make_record(i, i as f32));
        }
        assert_eq!(el.records().len(), 2);
        assert_eq!(el.total_record_count, 2); // Only 2 accepted
    }

    #[test]
    fn disable_logging() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        el.write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        el.add_record(make_record(10, 72.5));
        assert_eq!(el.records().len(), 0);
    }

    #[test]
    fn clear_buffer_via_record_count() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        el.add_record(make_record(10, 72.5));
        assert_eq!(el.records().len(), 1);
        el.write_property(
            PropertyIdentifier::RECORD_COUNT,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
        assert_eq!(el.records().len(), 0);
    }

    #[test]
    fn read_event_state_default() {
        let el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let val = el
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // normal
    }

    #[test]
    fn property_list_complete() {
        let el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let props = el.property_list();
        assert!(props.contains(&PropertyIdentifier::LOG_ENABLE));
        assert!(props.contains(&PropertyIdentifier::LOG_INTERVAL));
        assert!(props.contains(&PropertyIdentifier::STOP_WHEN_FULL));
        assert!(props.contains(&PropertyIdentifier::BUFFER_SIZE));
        assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
        assert!(props.contains(&PropertyIdentifier::RECORD_COUNT));
        assert!(props.contains(&PropertyIdentifier::TOTAL_RECORD_COUNT));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn write_log_interval() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        el.write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(60),
            None,
        )
        .unwrap();
        let val = el
            .read_property(PropertyIdentifier::LOG_INTERVAL, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(60));
    }

    #[test]
    fn write_unknown_property_denied() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let result = el.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn log_buffer_various_datum_types() {
        let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
        let date = make_date();
        let time = make_time(8);

        el.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::BooleanValue(true),
            status_flags: None,
        });
        el.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::EnumValue(42),
            status_flags: Some(0b0100),
        });
        el.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::NullValue,
            status_flags: None,
        });

        let val = el
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        if let PropertyValue::List(records) = val {
            assert_eq!(records.len(), 3);
            if let PropertyValue::List(fields) = &records[0] {
                assert_eq!(fields[2], PropertyValue::Boolean(true));
            } else {
                panic!("Expected List");
            }
            if let PropertyValue::List(fields) = &records[1] {
                assert_eq!(fields[2], PropertyValue::Enumerated(42));
            } else {
                panic!("Expected List");
            }
            if let PropertyValue::List(fields) = &records[2] {
                assert_eq!(fields[2], PropertyValue::Null);
            } else {
                panic!("Expected List");
            }
        } else {
            panic!("Expected List for LOG_BUFFER");
        }
    }
}
