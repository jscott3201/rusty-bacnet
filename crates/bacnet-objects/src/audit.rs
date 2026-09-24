//! AuditLog (type 61) and AuditReporter (type 62) objects per Addendum 135-2016bj.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogQueryParameters, BACnetAuditLogRecord,
    BACnetAuditLogRecordResult, BACnetAuditNotification, BACnetObjectSelector, BACnetRecipient,
};
use bacnet_types::enums::{
    AuditLevel, BACnetSuccessFilter, ErrorClass, ErrorCode, EventState, ObjectType,
    PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::clock::ClockReader;
use crate::common::read_property_list_property;
use crate::property_metadata::PropertyMetadata;
use crate::traits::BACnetObject;

mod association;
pub use association::{SelectedAuditReporter, TargetAuditAssociation};
mod object_policy;
pub use object_policy::{AuditPriorityPolicy, EffectiveAuditPolicy, ObjectAuditPolicy};

mod forwarding;
mod log_metadata;
pub use forwarding::AuditLogForwarding;
mod notification;
mod persistence;
mod receipt;
mod reporter_change;
mod reporter_metadata;
mod reporter_object;
mod reporter_status;
pub use notification::AuditLogNotificationSink;
use persistence::{validate_record, validate_snapshot};
pub use persistence::{
    AuditLogPersistence, AuditLogSnapshot, FileAuditLogPersistence, MAX_AUDIT_RECORDS,
};
pub use receipt::{
    CompletedAuditReceipt, ConfirmedAuditNotificationOutcome, MAX_AUDIT_RECEIPT_KEY_BYTES,
    MAX_COMPLETED_AUDIT_RECEIPTS,
};
pub use reporter_change::{
    AuditReporterAuthority, AuditReporterChangeSink, AuditReporterConfiguration,
};
pub use reporter_status::{AuditDeliveryToken, AuditReporterStatus};

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
    /// A present start is the corrected-baseline `Unsigned64` cursor
    /// (Errata 2024-04-29 item 8) and admits only literal sequence identities
    /// below it. This intentionally does not add a modular cursor across
    /// `u64::MAX -> 1`.
    fn query(
        &self,
        parameters: &BACnetAuditLogQueryParameters,
        start_at_sequence_number: Option<u64>,
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
    completed_receipts: Vec<CompletedAuditReceipt>,
    total_record_count: u64,
    status_flags: StatusFlags,
    forwarding: Option<Arc<AuditLogForwarding>>,
    generation: u64,
    persistence: Arc<dyn AuditLogPersistence>,
    clock: Option<Arc<dyn ClockReader>>,
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
                    completed_receipts: Vec::new(),
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
            completed_receipts: snapshot.completed_receipts,
            total_record_count: snapshot.total_record_count,
            status_flags: StatusFlags::empty(),
            forwarding: None,
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
            completed_receipts: self.completed_receipts.clone(),
        }
    }

    fn commit_and_apply(&mut self, snapshot: AuditLogSnapshot) -> Result<(), Error> {
        validate_snapshot(&snapshot)?;
        self.persistence.commit(&snapshot)?;
        self.generation = snapshot.generation;
        self.log_enable = snapshot.log_enable;
        self.total_record_count = snapshot.total_record_count;
        self.buffer = snapshot.records.into();
        self.completed_receipts = snapshot.completed_receipts;
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
    success_filter: BACnetSuccessFilter,
) -> bool {
    operations.is_none_or(|flags| flags.contains(notification.operation))
        // Corrected-baseline three-state filtering (RB-20, Errata 2024-04-29
        // item 7): ALL matches every outcome, SUCCESSES_ONLY matches records
        // without a result, and FAILURES_ONLY matches records with one. A
        // reserved raw value matches nothing rather than widening the query.
        && match success_filter.to_raw() {
            0 => true,
            1 => notification.result.is_none(),
            2 => notification.result.is_some(),
            _ => false,
        }
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
        start_at_sequence_number: Option<u64>,
        requested_count: u16,
    ) -> AuditLogQueryPage {
        let limit = usize::from(requested_count).min(MAX_AUDIT_RECORDS as usize);
        let mut records = Vec::with_capacity(limit.min(self.buffer.len()));
        let mut unreturned_match = false;

        // The ring is stored oldest-to-newest. Reverse insertion order is the
        // query order even across sequence wrap; numeric sorting would turn
        // retained [MAX, 1] into the wrong chronology.
        for result in self.buffer.iter().rev() {
            if start_at_sequence_number.is_some_and(|start| result.sequence_number >= start) {
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
            p if p == PropertyIdentifier::STATUS_FLAGS => {
                let mut flags = self.status_flags;
                flags.set(
                    StatusFlags::FAULT,
                    self.forwarding.as_ref().is_some_and(|forwarding| {
                        forwarding.status().reliability() != Reliability::NO_FAULT_DETECTED
                    }),
                );
                Ok(PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![flags.bits() << 4],
                })
            }
            p if self.forwarding.is_some() && p == PropertyIdentifier::MEMBER_OF => {
                let mut encoded = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_device_object_reference(
                    &mut encoded,
                    self.forwarding.as_ref().unwrap().parent(),
                );
                Ok(PropertyValue::ApplicationData(encoded.to_vec()))
            }
            p if self.forwarding.is_some() && p == PropertyIdentifier::DELETE_ON_FORWARD => {
                Ok(PropertyValue::Boolean(false))
            }
            p if self.forwarding.is_some()
                && p == PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS =>
            {
                Ok(PropertyValue::Boolean(true))
            }
            p if self.forwarding.is_some() && p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(
                    self.forwarding
                        .as_ref()
                        .unwrap()
                        .status()
                        .reliability()
                        .to_raw(),
                ))
            }
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

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        log_metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(
            log_metadata::for_object(self).as_ref(),
        )
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self)
    }

    fn audit_log_forwarding_internal(&self) -> Option<Arc<AuditLogForwarding>> {
        self.forwarding.clone()
    }

    fn set_audit_log_parent_internal(
        &mut self,
        parent: bacnet_types::constructed::BACnetDeviceObjectReference,
    ) -> Result<(), Error> {
        self.set_member_of(Some(parent));
        Ok(())
    }

    fn audit_log_notification_sink_internal(
        &mut self,
    ) -> Option<&mut dyn AuditLogNotificationSink> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// AuditReporter (type 62)
// ---------------------------------------------------------------------------

/// BACnet AuditReporter object — configures which audit notifications to send.
///
/// Ordinary Reporters are always targets. Source ownership is an endpoint-private
/// adapter, not a mutable flag or capability available from this crate.
///
/// ```compile_fail,E0599
/// use bacnet_objects::{audit::AuditReporterObject, database::ObjectDatabase, traits::BACnetObject};
/// let reporter = AuditReporterObject::new(1, "Reporter").unwrap();
/// let oid = reporter.object_identifier();
/// let mut db = ObjectDatabase::new();
/// db.add(Box::new(reporter)).unwrap();
/// AuditReporterObject::designate_source_internal(&mut db, oid).unwrap();
/// ```
///
/// ```compile_fail,E0432
/// use bacnet_objects::audit::SourceReporterBinding;
/// ```
pub struct AuditReporterObject {
    oid: ObjectIdentifier,
    name: String,
    status: Arc<AuditReporterStatus>,
    change_sink: Option<std::sync::Weak<dyn AuditReporterChangeSink>>,
    change_owner: std::sync::Weak<crate::database::AuditOwnership>,
    clock: Option<Arc<dyn ClockReader>>,
}

impl AuditReporterObject {
    /// Construct a disabled target Reporter, not yet bound to a server destination.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, instance)?,
            name: name.into(),
            status: Arc::new(AuditReporterStatus::default()),
            change_sink: None,
            change_owner: std::sync::Weak::new(),
            clock: None,
        })
    }
    /// Change Description through the installed target owner, if any.
    pub fn set_description(&mut self, desc: impl Into<String>) -> Result<(), Error> {
        AuditReporterAuthority(self).write_description(
            PropertyValue::CharacterString(desc.into()),
            None,
            None,
        )
    }
    /// Nominal selector membership; mandatory self-reporting is a separate election.
    #[doc(hidden)]
    pub fn monitors_object_internal(&self, target: ObjectIdentifier) -> bool {
        self.configuration_internal().monitors(target)
    }
    #[doc(hidden)]
    pub fn monitors_unassigned_create_internal(&self, kind: ObjectType) -> bool {
        self.configuration_internal().monitors_unassigned(kind)
    }
    /// Evaluate an already associated Reporter's WRITE filter.
    #[doc(hidden)]
    pub fn reports_write_internal(
        &self,
        property: PropertyIdentifier,
        command_priority: Option<u8>,
    ) -> bool {
        ObjectAuditPolicy::default()
            .effective_internal(&self.configuration_internal())
            .reports(
                bacnet_types::enums::AuditOperation::WRITE,
                Some(property),
                command_priority,
            )
    }
    /// Instance-owned configuration and delivery authority.
    #[doc(hidden)]
    pub fn status_internal(&self) -> Arc<AuditReporterStatus> {
        Arc::clone(&self.status)
    }
    #[doc(hidden)]
    pub fn confirmed_internal(&self) -> bool {
        self.status.confirmed()
    }
    fn reliability(&self) -> Reliability {
        if !self.configuration_internal().enabled() {
            Reliability::NO_FAULT_DETECTED
        } else {
            self.status.reliability()
        }
    }
}

#[cfg(test)]
#[path = "audit/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "audit/query_tests.rs"]
mod query_tests;

#[cfg(test)]
#[path = "audit/notification_tests.rs"]
mod notification_tests;

#[cfg(test)]
#[path = "audit/receipt_tests.rs"]
mod receipt_tests;

#[cfg(test)]
#[path = "audit/persistence_receipt_tests.rs"]
mod persistence_receipt_tests;

#[path = "audit/reporter_configuration.rs"]
mod reporter_configuration;
