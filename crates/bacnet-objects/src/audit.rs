//! AuditLog (type 62) and AuditReporter (type 61) objects per Addendum 135-2016bj.

use std::borrow::Cow;
use std::collections::VecDeque;

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::read_property_list_property;
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// AuditLog (type 62)
// ---------------------------------------------------------------------------

/// A single audit log record.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub timestamp_secs: u64,
    pub description: String,
}

/// BACnet AuditLog object — stores audit trail records in a ring buffer.
pub struct AuditLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    buffer_size: u32,
    buffer: VecDeque<AuditRecord>,
    total_record_count: u64,
    status_flags: StatusFlags,
}

impl AuditLogObject {
    pub fn new(instance: u32, name: impl Into<String>, buffer_size: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AUDIT_LOG, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            log_enable: true,
            buffer_size,
            buffer: VecDeque::new(),
            total_record_count: 0,
            status_flags: StatusFlags::empty(),
        })
    }

    /// Add an audit record to the log.
    pub fn add_record(&mut self, record: AuditRecord) {
        if !self.log_enable {
            return;
        }
        if self.buffer.len() >= self.buffer_size as usize {
            self.buffer.pop_front();
        }
        self.buffer.push_back(record);
        self.total_record_count += 1;
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<AuditRecord> {
        &self.buffer
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for AuditLogObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::AUDIT_LOG.to_raw()))
            }
            p if p == PropertyIdentifier::LOG_ENABLE => Ok(PropertyValue::Boolean(self.log_enable)),
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
        if property == PropertyIdentifier::RECORD_COUNT {
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
            PropertyIdentifier::BUFFER_SIZE,
            PropertyIdentifier::RECORD_COUNT,
            PropertyIdentifier::TOTAL_RECORD_COUNT,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AuditReporter (type 61)
// ---------------------------------------------------------------------------

/// BACnet AuditReporter object — configures which audit notifications to send.
pub struct AuditReporterObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
}

impl AuditReporterObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for AuditReporterObject {
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
                ObjectType::AUDIT_REPORTER.to_raw(),
            )),
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
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
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AuditLog ---

    #[test]
    fn audit_log_add_records() {
        let mut al = AuditLogObject::new(1, "AL-1", 100).unwrap();
        al.add_record(AuditRecord {
            timestamp_secs: 1000,
            description: "User login".into(),
        });
        assert_eq!(al.records().len(), 1);
        assert_eq!(
            al.read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
    }

    #[test]
    fn audit_log_ring_buffer() {
        let mut al = AuditLogObject::new(1, "AL-1", 2).unwrap();
        for i in 0..4 {
            al.add_record(AuditRecord {
                timestamp_secs: i * 60,
                description: format!("Event {i}"),
            });
        }
        assert_eq!(al.records().len(), 2);
        assert_eq!(al.records()[0].description, "Event 2");
        assert_eq!(
            al.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(4)
        );
    }

    #[test]
    fn audit_log_disable() {
        let mut al = AuditLogObject::new(1, "AL-1", 100).unwrap();
        al.write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        al.add_record(AuditRecord {
            timestamp_secs: 1000,
            description: "Should not appear".into(),
        });
        assert_eq!(al.records().len(), 0);
    }

    #[test]
    fn audit_log_clear() {
        let mut al = AuditLogObject::new(1, "AL-1", 100).unwrap();
        al.add_record(AuditRecord {
            timestamp_secs: 1000,
            description: "Event".into(),
        });
        al.write_property(
            PropertyIdentifier::RECORD_COUNT,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
        assert_eq!(al.records().len(), 0);
    }

    #[test]
    fn audit_log_read_object_type() {
        let al = AuditLogObject::new(1, "AL-1", 100).unwrap();
        assert_eq!(
            al.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::AUDIT_LOG.to_raw())
        );
    }

    // --- AuditReporter ---

    #[test]
    fn audit_reporter_read_object_type() {
        let ar = AuditReporterObject::new(1, "AR-1").unwrap();
        assert_eq!(
            ar.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::AUDIT_REPORTER.to_raw())
        );
    }

    #[test]
    fn audit_reporter_write_denied() {
        let mut ar = AuditReporterObject::new(1, "AR-1").unwrap();
        assert!(ar
            .write_property(
                PropertyIdentifier::OBJECT_NAME,
                None,
                PropertyValue::CharacterString("new".into()),
                None,
            )
            .is_err());
    }
}
