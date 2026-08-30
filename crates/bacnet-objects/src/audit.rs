//! AuditLog (type 61) and AuditReporter (type 62) objects per Addendum 135-2016bj.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogQueryParameters, BACnetAuditLogRecord,
    BACnetAuditLogRecordResult, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::clock::ClockReader;
use crate::common::read_property_list_property;
use crate::traits::{BACnetObject, WritePropertyRollback};

mod persistence;
use persistence::{validate_record, validate_snapshot};
pub use persistence::{
    AuditLogPersistence, AuditLogSnapshot, FileAuditLogPersistence, MAX_AUDIT_RECORDS,
};

/// One owned page returned by an object-level AuditLogQuery capability.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogQueryPage {
    /// Matching records in newest-first retained insertion order.
    pub records: Vec<BACnetAuditLogRecordResult>,
    /// Whether a complete retained-buffer scan found no unreturned match.
    pub no_more_items: bool,
}

/// Read-only query capability for an object's retained Audit Log records.
///
/// Implementations return owned pages so the server can release its object
/// database read guard before building and encoding the ComplexACK. This
/// interface never performs persistence I/O or mutates the log.
pub trait AuditLogStorage: Send + Sync {
    /// Filter and page the currently retained in-memory records.
    ///
    /// A present start is the existing Clause-21 `Unsigned32` model and admits
    /// only literal sequence identities below it. This intentionally does not
    /// add a modular cursor across `u64::MAX -> 1`.
    fn query(
        &self,
        parameters: &BACnetAuditLogQueryParameters,
        start_at_sequence_number: Option<u32>,
        requested_count: u16,
    ) -> AuditLogQueryPage;
}

// ---------------------------------------------------------------------------
// AuditLog (type 61)
// ---------------------------------------------------------------------------

/// BACnet AuditLog object with explicit application-owned durable storage.
pub struct AuditLogObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    log_enable: bool,
    buffer_size: u32,
    buffer: VecDeque<BACnetAuditLogRecordResult>,
    total_record_count: u64,
    status_flags: StatusFlags,
    generation: u64,
    persistence: Arc<dyn AuditLogPersistence>,
    clock: Option<Arc<dyn ClockReader>>,
}

struct AuditLogWriteRollback {
    snapshot: AuditLogSnapshot,
}

const LOG_DISABLED_STATUS: u8 = 0b001;
const BUFFER_PURGED_STATUS: u8 = 0b010;

impl AuditLogObject {
    /// Open or initialize one AuditLog using the explicitly supplied storage.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        buffer_size: u32,
        persistence: Arc<dyn AuditLogPersistence>,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AUDIT_LOG, instance)?;
        if buffer_size > MAX_AUDIT_RECORDS {
            return Err(Error::OutOfRange(format!(
                "AuditLog capacity {buffer_size} exceeds {MAX_AUDIT_RECORDS}"
            )));
        }
        let snapshot = match persistence.load(oid)? {
            Some(snapshot) => {
                if snapshot.object_identifier != oid || snapshot.capacity != buffer_size {
                    return Err(Error::Encoding(
                        "AuditLog persisted identity or capacity does not match configuration"
                            .into(),
                    ));
                }
                validate_snapshot(&snapshot)?;
                snapshot
            }
            None => {
                let snapshot = AuditLogSnapshot {
                    object_identifier: oid,
                    generation: 1,
                    capacity: buffer_size,
                    log_enable: true,
                    total_record_count: 0,
                    records: Vec::new(),
                };
                validate_snapshot(&snapshot)?;
                persistence.commit(&snapshot)?;
                snapshot
            }
        };
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            log_enable: snapshot.log_enable,
            buffer_size: snapshot.capacity,
            buffer: snapshot.records.into(),
            total_record_count: snapshot.total_record_count,
            status_flags: StatusFlags::empty(),
            generation: snapshot.generation,
            persistence,
            clock: None,
        })
    }

    /// Append one application-supplied record when logging is enabled.
    pub fn add_record(&mut self, record: BACnetAuditLogRecord) -> Result<Option<u64>, Error> {
        if !self.log_enable {
            return Ok(None);
        }
        validate_record(&record)?;
        let mut prospective = self.snapshot_for_next_generation()?;
        let sequence_number = append_record(&mut prospective, record);
        self.commit_and_apply(prospective)?;
        Ok(Some(sequence_number))
    }

    /// Get the current buffer contents.
    pub fn records(&self) -> &VecDeque<BACnetAuditLogRecordResult> {
        &self.buffer
    }

    /// Configured and persisted ring capacity.
    pub fn buffer_size(&self) -> u32 {
        self.buffer_size
    }

    /// Persisted logging enable policy.
    pub fn log_enable(&self) -> bool {
        self.log_enable
    }

    /// Monotonic record identity counter with BACnet MAX-to-one wrap.
    pub fn total_record_count(&self) -> u64 {
        self.total_record_count
    }

    /// Current durable snapshot generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Clear buffered records and append the internal BUFFER_PURGED status.
    fn purge(&mut self) -> Result<u64, Error> {
        let timestamp = self.valid_timestamp()?;
        let mut prospective = self.snapshot_for_next_generation()?;
        prospective.records.clear();
        let sequence_number = append_record(
            &mut prospective,
            BACnetAuditLogRecord {
                timestamp,
                datum: BACnetAuditLogDatum::LogStatus(BUFFER_PURGED_STATUS),
            },
        );
        self.commit_and_apply(prospective)?;
        Ok(sequence_number)
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn valid_timestamp(
        &self,
    ) -> Result<
        (
            bacnet_types::primitives::Date,
            bacnet_types::primitives::Time,
        ),
        Error,
    > {
        let frame = self
            .clock
            .as_ref()
            .and_then(|clock| clock.read_clock())
            .filter(|frame| frame.is_valid_actual_datetime())
            .ok_or(Error::Protocol {
                class: ErrorClass::DEVICE.to_raw() as u32,
                code: ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32,
            })?;
        Ok((frame.local_date, frame.local_time))
    }

    fn snapshot_for_next_generation(&self) -> Result<AuditLogSnapshot, Error> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::OutOfRange("AuditLog persistence generation exhausted".into()))?;
        let mut snapshot = self.current_snapshot();
        snapshot.generation = generation;
        Ok(snapshot)
    }

    fn current_snapshot(&self) -> AuditLogSnapshot {
        AuditLogSnapshot {
            object_identifier: self.oid,
            generation: self.generation,
            capacity: self.buffer_size,
            log_enable: self.log_enable,
            total_record_count: self.total_record_count,
            records: self.buffer.iter().cloned().collect(),
        }
    }

    fn commit_and_apply(&mut self, snapshot: AuditLogSnapshot) -> Result<(), Error> {
        validate_snapshot(&snapshot)?;
        self.persistence.commit(&snapshot)?;
        self.generation = snapshot.generation;
        self.log_enable = snapshot.log_enable;
        self.total_record_count = snapshot.total_record_count;
        self.buffer = snapshot.records.into();
        Ok(())
    }
}

fn append_record(snapshot: &mut AuditLogSnapshot, record: BACnetAuditLogRecord) -> u64 {
    let sequence_number = if snapshot.total_record_count == u64::MAX {
        1
    } else {
        snapshot.total_record_count + 1
    };
    snapshot.total_record_count = sequence_number;
    if snapshot.capacity != 0 {
        if snapshot.records.len() >= snapshot.capacity as usize {
            snapshot.records.remove(0);
        }
        snapshot.records.push(BACnetAuditLogRecordResult {
            sequence_number,
            record,
        });
    }
    sequence_number
}

fn recipient_matches(
    actual: &BACnetRecipient,
    required_identifier: ObjectIdentifier,
    optional_address: Option<&bacnet_types::constructed::BACnetAddress>,
) -> bool {
    match actual {
        BACnetRecipient::Device(identifier) => *identifier == required_identifier,
        BACnetRecipient::Address(address) => {
            optional_address.is_some_and(|filter| address == filter)
        }
    }
}

fn operation_matches(
    notification: &BACnetAuditNotification,
    operations: Option<bacnet_types::bitstring::AuditOperationFlags>,
    successful_actions_only: bool,
) -> bool {
    operations.is_none_or(|flags| flags.contains(notification.operation))
        && (!successful_actions_only || notification.result.is_none())
}

fn query_matches(
    notification: &BACnetAuditNotification,
    parameters: &BACnetAuditLogQueryParameters,
) -> bool {
    match parameters {
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address,
            target_object_identifier,
            target_property_identifier,
            target_array_index,
            target_priority,
            operations,
            successful_actions_only,
        } => {
            recipient_matches(
                &notification.target_device,
                *target_device_identifier,
                target_device_address.as_ref(),
            ) && target_object_identifier
                .is_none_or(|filter| notification.target_object == Some(filter))
                && target_property_identifier.is_none_or(|filter| {
                    notification.target_property.as_ref().is_some_and(|property| {
                        property.property_identifier == filter
                    })
                })
                && target_array_index.is_none_or(|filter| {
                    notification.target_property.as_ref().is_some_and(|property| {
                        property.property_array_index == Some(filter)
                    })
                })
                // Clause 13.19 says a record without Priority matches any
                // requested Target Priority.
                && target_priority.is_none_or(|filter| {
                    notification
                        .target_priority
                        .is_none_or(|priority| priority == filter)
                })
                && operation_matches(notification, *operations, *successful_actions_only)
        }
        BACnetAuditLogQueryParameters::BySource {
            source_device_identifier,
            source_device_address,
            source_object_identifier,
            operations,
            successful_actions_only,
        } => {
            recipient_matches(
                &notification.source_device,
                *source_device_identifier,
                source_device_address.as_ref(),
            ) && source_object_identifier
                .is_none_or(|filter| notification.source_object == Some(filter))
                && operation_matches(notification, *operations, *successful_actions_only)
        }
    }
}

impl AuditLogStorage for AuditLogObject {
    fn query(
        &self,
        parameters: &BACnetAuditLogQueryParameters,
        start_at_sequence_number: Option<u32>,
        requested_count: u16,
    ) -> AuditLogQueryPage {
        let limit = usize::from(requested_count).min(MAX_AUDIT_RECORDS as usize);
        let mut records = Vec::with_capacity(limit.min(self.buffer.len()));
        let mut unreturned_match = false;

        // The ring is stored oldest-to-newest. Reverse insertion order is the
        // query order even across sequence wrap; numeric sorting would turn
        // retained [MAX, 1] into the wrong chronology.
        for result in self.buffer.iter().rev() {
            if start_at_sequence_number
                .is_some_and(|start| result.sequence_number >= u64::from(start))
            {
                continue;
            }
            let BACnetAuditLogDatum::AuditNotification(notification) = &result.record.datum else {
                continue;
            };
            if !query_matches(notification, parameters) {
                continue;
            }
            if records.len() < limit {
                records.push(result.clone());
            } else {
                // Keep scanning the complete retained snapshot so a full page
                // can still truthfully distinguish exhaustion from a later
                // eligible match. This also defines count=0.
                unreturned_match = true;
            }
        }

        AuditLogQueryPage {
            records,
            no_more_items: !unreturned_match,
        }
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
                if v == self.log_enable {
                    return Ok(());
                }
                let timestamp = self.valid_timestamp()?;
                let mut prospective = self.snapshot_for_next_generation()?;
                prospective.log_enable = v;
                append_record(
                    &mut prospective,
                    BACnetAuditLogRecord {
                        timestamp,
                        datum: BACnetAuditLogDatum::LogStatus(if v {
                            0
                        } else {
                            LOG_DISABLED_STATUS
                        }),
                    },
                );
                self.commit_and_apply(prospective)?;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::RECORD_COUNT {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
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

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self)
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        let PropertyValue::Boolean(requested) = value else {
            return None;
        };
        (property == PropertyIdentifier::LOG_ENABLE && *requested != self.log_enable).then(|| {
            WritePropertyRollback::new(AuditLogWriteRollback {
                snapshot: self.current_snapshot(),
            })
        })
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        let mut snapshot = rollback.downcast::<AuditLogWriteRollback>()?.snapshot;
        if snapshot.object_identifier != self.oid || snapshot.capacity != self.buffer_size {
            return Err(Error::Encoding(
                "AuditLog rollback snapshot does not belong to this object".into(),
            ));
        }
        snapshot.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::OutOfRange("AuditLog persistence generation exhausted".into()))?;
        self.commit_and_apply(snapshot)
    }
}

// ---------------------------------------------------------------------------
// AuditReporter (type 62)
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
#[path = "audit/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "audit/query_tests.rs"]
mod query_tests;
