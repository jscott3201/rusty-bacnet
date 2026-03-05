//! TrendLog (type 20) and TrendLogMultiple (type 27) objects per ASHRAE 135-2020.

use std::borrow::Cow;
use std::collections::VecDeque;

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_property_list_property};
use crate::traits::BACnetObject;

/// BACnet TrendLog object.
///
/// Ring buffer of timestamped property values. The application calls
/// `add_record()` to log values at `log_interval` intervals.
pub struct TrendLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    log_interval: u32,
    stop_when_full: bool,
    buffer_size: u32,
    buffer: VecDeque<BACnetLogRecord>,
    total_record_count: u64,
    out_of_service: bool,
    reliability: u32,
    status_flags: StatusFlags,
    log_device_object_property: Option<BACnetDeviceObjectPropertyReference>,
    logging_type: u32, // 0=polled, 1=cov, 2=triggered
}

impl TrendLogObject {
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TREND_LOG, instance)?;
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
            out_of_service: false,
            reliability: 0,
            status_flags: StatusFlags::empty(),
            log_device_object_property: None,
            logging_type: 0,
        })
    }

    /// Add a BACnetLogRecord to the trend log buffer.
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

    /// Set the log device object property reference.
    pub fn set_log_device_object_property(
        &mut self,
        reference: Option<BACnetDeviceObjectPropertyReference>,
    ) {
        self.log_device_object_property = reference;
    }

    /// Set the logging type (0=polled, 1=cov, 2=triggered).
    pub fn set_logging_type(&mut self, logging_type: u32) {
        self.logging_type = logging_type;
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
            p if p == PropertyIdentifier::LOG_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.log_interval as u64))
            }
            p if p == PropertyIdentifier::STOP_WHEN_FULL => {
                Ok(PropertyValue::Boolean(self.stop_when_full))
            }
            p if p == PropertyIdentifier::BUFFER_SIZE => {
                Ok(PropertyValue::Unsigned(self.buffer_size as u64))
            }
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.buffer.len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.total_record_count))
            }
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability))
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::LOG_BUFFER => {
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
                            LogDatum::BitstringValue { unused_bits, data } => {
                                PropertyValue::BitString {
                                    unused_bits: *unused_bits,
                                    data: data.clone(),
                                }
                            }
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
                Ok(PropertyValue::List(records))
            }
            p if p == PropertyIdentifier::LOGGING_TYPE => {
                Ok(PropertyValue::Enumerated(self.logging_type))
            }
            p if p == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY => {
                match &self.log_device_object_property {
                    None => Ok(PropertyValue::Null),
                    Some(r) => Ok(PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(r.object_identifier),
                        PropertyValue::Unsigned(r.property_identifier as u64),
                        match r.property_array_index {
                            Some(idx) => PropertyValue::Unsigned(idx as u64),
                            None => PropertyValue::Null,
                        },
                        match r.device_identifier {
                            Some(dev) => PropertyValue::ObjectIdentifier(dev),
                            None => PropertyValue::Null,
                        },
                    ])),
                }
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
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::LOG_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.log_enable = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::LOG_INTERVAL {
            if let PropertyValue::Unsigned(v) = value {
                self.log_interval = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::STOP_WHEN_FULL {
            if let PropertyValue::Boolean(v) = value {
                self.stop_when_full = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            // Writing 0 clears the buffer
            if let PropertyValue::Unsigned(0) = value {
                self.buffer.clear();
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::OUT_OF_SERVICE {
            if let PropertyValue::Boolean(v) = value {
                self.out_of_service = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(s) = value {
                self.description = s;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
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
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::LOGGING_TYPE,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn add_trend_record(&mut self, record: BACnetLogRecord) {
        self.add_record(record);
    }
}

// ---------------------------------------------------------------------------
// TrendLogMultiple (type 27)
// ---------------------------------------------------------------------------

/// BACnet TrendLogMultiple object (type 27).
///
/// Multi-channel trending. Logs values from multiple properties simultaneously.
/// Unlike TrendLog which monitors a single property, TrendLogMultiple monitors
/// a list of device-object-property references per record.
pub struct TrendLogMultipleObject {
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
    log_device_object_property: Vec<BACnetDeviceObjectPropertyReference>,
    logging_type: u32, // 0=polled, 1=cov, 2=triggered
    out_of_service: bool,
    reliability: u32,
}

impl TrendLogMultipleObject {
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, instance)?;
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
            log_device_object_property: Vec::new(),
            logging_type: 0,
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a BACnetLogRecord to the trend log buffer.
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

    /// Add a property reference to the monitored list.
    pub fn add_property_reference(&mut self, reference: BACnetDeviceObjectPropertyReference) {
        self.log_device_object_property.push(reference);
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

    /// Set the logging type (0=polled, 1=cov, 2=triggered).
    pub fn set_logging_type(&mut self, logging_type: u32) {
        self.logging_type = logging_type;
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
            p if p == PropertyIdentifier::LOG_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.log_interval as u64))
            }
            p if p == PropertyIdentifier::STOP_WHEN_FULL => {
                Ok(PropertyValue::Boolean(self.stop_when_full))
            }
            p if p == PropertyIdentifier::BUFFER_SIZE => {
                Ok(PropertyValue::Unsigned(self.buffer_size as u64))
            }
            p if p == PropertyIdentifier::RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.buffer.len() as u64))
            }
            p if p == PropertyIdentifier::TOTAL_RECORD_COUNT => {
                Ok(PropertyValue::Unsigned(self.total_record_count))
            }
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability))
            }
            p if p == PropertyIdentifier::LOG_BUFFER => {
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
                            LogDatum::BitstringValue { unused_bits, data } => {
                                PropertyValue::BitString {
                                    unused_bits: *unused_bits,
                                    data: data.clone(),
                                }
                            }
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
                Ok(PropertyValue::List(records))
            }
            p if p == PropertyIdentifier::LOGGING_TYPE => {
                Ok(PropertyValue::Enumerated(self.logging_type))
            }
            p if p == PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY => {
                let refs: Vec<PropertyValue> = self
                    .log_device_object_property
                    .iter()
                    .map(|r| {
                        PropertyValue::List(vec![
                            PropertyValue::ObjectIdentifier(r.object_identifier),
                            PropertyValue::Unsigned(r.property_identifier as u64),
                            match r.property_array_index {
                                Some(idx) => PropertyValue::Unsigned(idx as u64),
                                None => PropertyValue::Null,
                            },
                            match r.device_identifier {
                                Some(dev) => PropertyValue::ObjectIdentifier(dev),
                                None => PropertyValue::Null,
                            },
                        ])
                    })
                    .collect();
                Ok(PropertyValue::List(refs))
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
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::LOG_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.log_enable = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::LOG_INTERVAL {
            if let PropertyValue::Unsigned(v) = value {
                self.log_interval = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::STOP_WHEN_FULL {
            if let PropertyValue::Boolean(v) = value {
                self.stop_when_full = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            // Writing 0 clears the buffer
            if let PropertyValue::Unsigned(0) = value {
                self.buffer.clear();
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(s) = value {
                self.description = s;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
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
            PropertyIdentifier::LOGGING_TYPE,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn add_trend_record(&mut self, record: BACnetLogRecord) {
        self.add_record(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::primitives::{Date, Time};

    fn make_record(hour: u8, value: f32) -> BACnetLogRecord {
        BACnetLogRecord {
            date: Date {
                year: 124,
                month: 3,
                day: 15,
                day_of_week: 5,
            },
            time: Time {
                hour,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::RealValue(value),
            status_flags: None,
        }
    }

    #[test]
    fn trendlog_add_records() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.add_record(make_record(10, 72.5));
        tl.add_record(make_record(11, 73.0));
        assert_eq!(tl.records().len(), 2);
        let val = tl
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(2));
        let val = tl
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(2));
    }

    #[test]
    fn trendlog_ring_buffer_wraps() {
        let mut tl = TrendLogObject::new(1, "TL-1", 3).unwrap();
        for i in 0..5u8 {
            tl.add_record(BACnetLogRecord {
                date: Date {
                    year: 124,
                    month: 3,
                    day: 15,
                    day_of_week: 5,
                },
                time: Time {
                    hour: i,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                log_datum: LogDatum::UnsignedValue(i as u64),
                status_flags: None,
            });
        }
        assert_eq!(tl.records().len(), 3);
        // Oldest records should have been evicted; first remaining is hour=2
        assert_eq!(tl.records()[0].time.hour, 2);
        let val = tl
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(5));
    }

    #[test]
    fn trendlog_stop_when_full() {
        let mut tl = TrendLogObject::new(1, "TL-1", 2).unwrap();
        tl.write_property(
            PropertyIdentifier::STOP_WHEN_FULL,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        for i in 0..5u8 {
            tl.add_record(make_record(i, i as f32));
        }
        assert_eq!(tl.records().len(), 2);
        assert_eq!(tl.total_record_count, 2); // Only 2 accepted
    }

    #[test]
    fn trendlog_disable_logging() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        tl.add_record(make_record(10, 72.5));
        assert_eq!(tl.records().len(), 0);
    }

    #[test]
    fn trendlog_clear_buffer() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.add_record(make_record(10, 72.5));
        assert_eq!(tl.records().len(), 1);
        tl.write_property(
            PropertyIdentifier::RECORD_COUNT,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
        assert_eq!(tl.records().len(), 0);
    }

    #[test]
    fn trendlog_read_object_type() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        let val = tl
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::TREND_LOG.to_raw())
        );
    }

    #[test]
    fn trendlog_description_read_write() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        // Default is empty string
        assert_eq!(
            tl.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString(String::new())
        );
        tl.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Zone temperature trend".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            tl.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Zone temperature trend".into())
        );
    }

    #[test]
    fn trendlog_set_description_convenience() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.set_description("Outdoor air temperature log");
        assert_eq!(
            tl.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Outdoor air temperature log".into())
        );
    }

    #[test]
    fn trendlog_description_in_property_list() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        assert!(tl
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn trendlog_read_log_buffer() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.add_record(make_record(10, 72.5));
        tl.add_record(make_record(11, 73.0));
        let val = tl
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        if let PropertyValue::List(records) = val {
            assert_eq!(records.len(), 2);
            // First record
            if let PropertyValue::List(fields) = &records[0] {
                assert_eq!(fields.len(), 3);
                assert_eq!(fields[0], PropertyValue::Date(make_record(10, 72.5).date));
                assert_eq!(fields[1], PropertyValue::Time(make_record(10, 72.5).time));
                assert_eq!(fields[2], PropertyValue::Real(72.5));
            } else {
                panic!("Expected List for log record");
            }
            // Second record
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
    fn trendlog_log_buffer_empty() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        let val = tl
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn trendlog_log_buffer_overflow_stop_when_full() {
        let mut tl = TrendLogObject::new(1, "TL-1", 3).unwrap();
        tl.write_property(
            PropertyIdentifier::STOP_WHEN_FULL,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        for i in 0..5u8 {
            tl.add_record(make_record(i, i as f32 * 10.0));
        }
        // Buffer capped at 3; only first 3 records accepted
        let val = tl
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        if let PropertyValue::List(records) = val {
            assert_eq!(records.len(), 3);
            if let PropertyValue::List(fields) = &records[0] {
                assert_eq!(fields[2], PropertyValue::Real(0.0));
            } else {
                panic!("Expected List");
            }
            if let PropertyValue::List(fields) = &records[2] {
                assert_eq!(fields[2], PropertyValue::Real(20.0));
            } else {
                panic!("Expected List");
            }
        } else {
            panic!("Expected List for LOG_BUFFER");
        }
    }

    #[test]
    fn trendlog_read_logging_type() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        let val = tl
            .read_property(PropertyIdentifier::LOGGING_TYPE, None)
            .unwrap();
        // Default is 0 (polled)
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn trendlog_set_logging_type() {
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        tl.set_logging_type(1); // COV
        let val = tl
            .read_property(PropertyIdentifier::LOGGING_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
    }

    #[test]
    fn trendlog_log_buffer_in_property_list() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        let props = tl.property_list();
        assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
        assert!(props.contains(&PropertyIdentifier::LOGGING_TYPE));
        assert!(props.contains(&PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY));
    }

    #[test]
    fn trendlog_log_device_object_property_null_by_default() {
        let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
        let val = tl
            .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Null);
    }

    #[test]
    fn trendlog_log_buffer_various_datum_types() {
        use bacnet_types::constructed::LogDatum;
        let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();

        let date = Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        };
        let time = Time {
            hour: 8,
            minute: 0,
            second: 0,
            hundredths: 0,
        };

        tl.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::BooleanValue(true),
            status_flags: None,
        });
        tl.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::EnumValue(42),
            status_flags: Some(0b0100),
        });
        tl.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::NullValue,
            status_flags: None,
        });

        let val = tl
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

    // -----------------------------------------------------------------------
    // TrendLogMultiple tests
    // -----------------------------------------------------------------------

    #[test]
    fn trendlog_multiple_create() {
        let tlm = TrendLogMultipleObject::new(1, "TLM-1", 200).unwrap();
        assert_eq!(
            tlm.read_property(PropertyIdentifier::OBJECT_NAME, None)
                .unwrap(),
            PropertyValue::CharacterString("TLM-1".into())
        );
        assert_eq!(
            tlm.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::TREND_LOG_MULTIPLE.to_raw())
        );
        assert_eq!(
            tlm.read_property(PropertyIdentifier::BUFFER_SIZE, None)
                .unwrap(),
            PropertyValue::Unsigned(200)
        );
    }

    #[test]
    fn trendlog_multiple_add_records() {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
        tlm.add_record(make_record(10, 72.5));
        tlm.add_record(make_record(11, 73.0));
        assert_eq!(tlm.records().len(), 2);
        assert_eq!(
            tlm.read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert_eq!(
            tlm.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
    }

    #[test]
    fn trendlog_multiple_ring_buffer() {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap();
        for i in 0..5u8 {
            tlm.add_record(BACnetLogRecord {
                date: Date {
                    year: 124,
                    month: 3,
                    day: 15,
                    day_of_week: 5,
                },
                time: Time {
                    hour: i,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                log_datum: LogDatum::UnsignedValue(i as u64),
                status_flags: None,
            });
        }
        assert_eq!(tlm.records().len(), 3);
        assert_eq!(tlm.records()[0].time.hour, 2);
        assert_eq!(
            tlm.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(5)
        );
    }

    #[test]
    fn trendlog_multiple_read_log_buffer() {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
        tlm.add_record(make_record(10, 72.5));
        let val = tlm
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .unwrap();
        if let PropertyValue::List(records) = val {
            assert_eq!(records.len(), 1);
            if let PropertyValue::List(fields) = &records[0] {
                assert_eq!(fields[2], PropertyValue::Real(72.5));
            } else {
                panic!("Expected List for log record");
            }
        } else {
            panic!("Expected List for LOG_BUFFER");
        }
    }

    #[test]
    fn trendlog_multiple_property_list() {
        let tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
        let props = tlm.property_list();
        assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
        assert!(props.contains(&PropertyIdentifier::LOGGING_TYPE));
        assert!(props.contains(&PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn trendlog_multiple_add_property_references() {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();

        let oid1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let oid2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();

        tlm.add_property_reference(BACnetDeviceObjectPropertyReference {
            object_identifier: oid1,
            property_identifier: pv_raw,
            property_array_index: None,
            device_identifier: None,
        });
        tlm.add_property_reference(BACnetDeviceObjectPropertyReference {
            object_identifier: oid2,
            property_identifier: pv_raw,
            property_array_index: Some(3),
            device_identifier: None,
        });

        let val = tlm
            .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
            .unwrap();
        if let PropertyValue::List(refs) = val {
            assert_eq!(refs.len(), 2);
            // First reference
            if let PropertyValue::List(fields) = &refs[0] {
                assert_eq!(fields[0], PropertyValue::ObjectIdentifier(oid1));
                assert_eq!(fields[1], PropertyValue::Unsigned(pv_raw as u64));
                assert_eq!(fields[2], PropertyValue::Null);
                assert_eq!(fields[3], PropertyValue::Null);
            } else {
                panic!("Expected List for property reference");
            }
            // Second reference with array index
            if let PropertyValue::List(fields) = &refs[1] {
                assert_eq!(fields[0], PropertyValue::ObjectIdentifier(oid2));
                assert_eq!(fields[1], PropertyValue::Unsigned(pv_raw as u64));
                assert_eq!(fields[2], PropertyValue::Unsigned(3));
                assert_eq!(fields[3], PropertyValue::Null);
            } else {
                panic!("Expected List for property reference");
            }
        } else {
            panic!("Expected List for LOG_DEVICE_OBJECT_PROPERTY");
        }
    }

    #[test]
    fn trendlog_multiple_empty_property_references() {
        let tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
        let val = tlm
            .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn trendlog_multiple_write_log_enable() {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
        tlm.write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        assert_eq!(
            tlm.read_property(PropertyIdentifier::LOG_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        // Records should not be added when disabled
        tlm.add_record(make_record(10, 72.5));
        assert_eq!(tlm.records().len(), 0);
    }
}
