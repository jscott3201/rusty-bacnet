//! Dependency-neutral Audit service and Audit Log base models.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::bitstring::AuditOperationFlags;
use crate::enums::{AuditOperation, ErrorClass, ErrorCode, PropertyIdentifier};
use crate::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};

use super::{BACnetAddress, BACnetRecipient};

/// One audit operation record (`BACnetAuditNotification`, Clause 21).
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetAuditNotification {
    /// Optional timestamp at the source device.
    pub source_timestamp: Option<BACnetTimeStamp>,
    /// Optional timestamp at the target device.
    pub target_timestamp: Option<BACnetTimeStamp>,
    /// Device or address that originated the operation.
    pub source_device: BACnetRecipient,
    /// Optional source object.
    pub source_object: Option<ObjectIdentifier>,
    /// Operation being audited.
    pub operation: AuditOperation,
    /// Optional source-side comment.
    pub source_comment: Option<String>,
    /// Optional target-side comment.
    pub target_comment: Option<String>,
    /// Optional originating service invoke identifier.
    pub invoke_id: Option<u8>,
    /// Optional source user identifier.
    pub source_user_id: Option<u16>,
    /// Optional source user role.
    pub source_user_role: Option<u8>,
    /// Device or address targeted by the operation.
    pub target_device: BACnetRecipient,
    /// Optional target object.
    pub target_object: Option<ObjectIdentifier>,
    /// Optional target property reference.
    pub target_property: Option<AuditPropertyReference>,
    /// Command priority, constrained to `1..=16` when present.
    pub target_priority: Option<u8>,
    /// Raw, structurally validated `ABSTRACT-SYNTAX.&Type` encoding.
    pub target_value: Option<Vec<u8>>,
    /// Raw, structurally validated `ABSTRACT-SYNTAX.&Type` encoding.
    pub current_value: Option<Vec<u8>>,
    /// Optional BACnet error produced by the audited operation.
    pub result: Option<(ErrorClass, ErrorCode)>,
}

/// One result returned by AuditLogQuery.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetAuditLogRecordResult {
    /// Stable sequence identity assigned when the record was generated.
    pub sequence_number: u64,
    /// Typed Audit Log record.
    pub record: BACnetAuditLogRecord,
}

/// One Audit Log record, distinct from Trend/Event Log record models.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetAuditLogRecord {
    /// BACnetDateTime encoded as application Date followed by application Time.
    pub timestamp: (Date, Time),
    /// Audit-specific record datum.
    pub datum: BACnetAuditLogDatum,
}

/// Audit-specific `BACnetAuditLogRecord` datum CHOICE.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Preserve the existing direct public variant shape.
pub enum BACnetAuditLogDatum {
    /// Three-bit BACnetLogStatus: log-disabled, buffer-purged, log-interrupted.
    LogStatus(u8),
    /// A bare BACnetAuditNotification wrapped by choice tag `[1]`.
    AuditNotification(BACnetAuditNotification),
    /// Clock adjustment in seconds, encoded as a four-octet REAL under `[2]`.
    TimeChange(f32),
}

/// Typed `BACnetAuditLogQueryParameters` CHOICE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BACnetAuditLogQueryParameters {
    /// Match operations by target attributes.
    ByTarget {
        /// Required target device identity.
        target_device_identifier: ObjectIdentifier,
        /// Optional target device network address.
        target_device_address: Option<BACnetAddress>,
        /// Optional target object identity.
        target_object_identifier: Option<ObjectIdentifier>,
        /// Optional target property identity.
        target_property_identifier: Option<PropertyIdentifier>,
        /// Optional target property array index.
        target_array_index: Option<u64>,
        /// Command priority filter, constrained to `1..=16` when present.
        target_priority: Option<u8>,
        /// Optional operation bit filter.
        operations: Option<AuditOperationFlags>,
        /// Whether only successful operations match.
        successful_actions_only: bool,
    },
    /// Match operations by source attributes.
    BySource {
        /// Required source device identity.
        source_device_identifier: ObjectIdentifier,
        /// Optional source device network address.
        source_device_address: Option<BACnetAddress>,
        /// Optional source object identity.
        source_object_identifier: Option<ObjectIdentifier>,
        /// Optional operation bit filter.
        operations: Option<AuditOperationFlags>,
        /// Whether only successful operations match.
        successful_actions_only: bool,
    },
}

/// Audit-local wire-equivalent of `BACnetPropertyReference`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPropertyReference {
    /// Property selected by the Audit notification.
    pub property_identifier: PropertyIdentifier,
    /// Optional array index, within the primitive layer's `u64` domain.
    pub property_array_index: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_record_models_are_dependency_neutral_and_exactly_typed() {
        let record = BACnetAuditLogRecordResult {
            sequence_number: u64::MAX,
            record: BACnetAuditLogRecord {
                timestamp: (
                    Date {
                        year: 124,
                        month: 2,
                        day: 29,
                        day_of_week: 4,
                    },
                    Time {
                        hour: 12,
                        minute: 0,
                        second: 0,
                        hundredths: 0,
                    },
                ),
                datum: BACnetAuditLogDatum::TimeChange(-1.5),
            },
        };
        assert_eq!(record.clone(), record);
    }
}
