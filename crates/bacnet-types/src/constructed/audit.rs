//! Dependency-neutral Audit service and Audit Log base models.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::bitstring::AuditOperationFlags;
use crate::enums::{
    AuditOperation, BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use crate::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};

use super::{BACnetAddress, BACnetRecipient};

/// `BACnetObjectSelector` (Clause 21), used by Audit Reporter Monitored_Objects.
///
/// This CHOICE uses application tags, not context tags. NULL is an ignored
/// entry, not a wildcard; ObjectType includes the repository's extensible values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BACnetObjectSelector {
    /// Ignore this array entry.
    None,
    /// Select exactly one object.
    Object(ObjectIdentifier),
    /// Select all objects of this type.
    ObjectType(ObjectType),
}

impl BACnetObjectSelector {
    /// Project to the primitive application-tagged codec representation.
    pub fn encode_property_value(&self) -> PropertyValue {
        match self {
            Self::None => PropertyValue::Null,
            Self::Object(object) => PropertyValue::ObjectIdentifier(*object),
            Self::ObjectType(kind) => PropertyValue::Enumerated(kind.to_raw()),
        }
    }

    /// Decode one application value, rejecting alternatives outside the CHOICE.
    pub fn decode_property_value(value: &PropertyValue) -> Result<Self, crate::error::Error> {
        match value {
            PropertyValue::Null => Ok(Self::None),
            PropertyValue::ObjectIdentifier(object) => Ok(Self::Object(*object)),
            PropertyValue::Enumerated(kind) => Ok(Self::ObjectType(ObjectType::from_raw(*kind))),
            _ => Err(crate::error::Error::Encoding(
                "BACnetObjectSelector requires NULL, ObjectIdentifier, or Enumerated".into(),
            )),
        }
    }
}

#[cfg(test)]
mod selector_tests {
    use super::*;

    #[test]
    fn audit_object_selector_preserves_choice_and_extensible_type() {
        let object = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 42).unwrap();
        for (selector, value) in [
            (BACnetObjectSelector::None, PropertyValue::Null),
            (
                BACnetObjectSelector::Object(object),
                PropertyValue::ObjectIdentifier(object),
            ),
            (
                BACnetObjectSelector::ObjectType(ObjectType::BINARY_VALUE),
                PropertyValue::Enumerated(5),
            ),
            (
                BACnetObjectSelector::ObjectType(ObjectType::from_raw(128)),
                PropertyValue::Enumerated(128),
            ),
        ] {
            assert_eq!(selector.encode_property_value(), value);
            assert_eq!(
                BACnetObjectSelector::decode_property_value(&value).unwrap(),
                selector
            );
        }
        for invalid in [
            PropertyValue::Unsigned(5),
            PropertyValue::Boolean(false),
            PropertyValue::List(vec![]),
        ] {
            assert!(matches!(
                BACnetObjectSelector::decode_property_value(&invalid),
                Err(crate::error::Error::Encoding(_))
            ));
        }
    }
}

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
        /// Which operation outcomes match: all, successes-only, or
        /// failures-only (`BACnetSuccessFilter`, Clause 21.6 tags [7]/[4]).
        successful_actions_only: BACnetSuccessFilter,
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
        /// Which operation outcomes match: all, successes-only, or
        /// failures-only (`BACnetSuccessFilter`, Clause 21.6 tags [7]/[4]).
        successful_actions_only: BACnetSuccessFilter,
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

    #[test]
    #[allow(deprecated)]
    fn legacy_bool_helper_maps_old_meaning_to_named_filter() {
        assert_eq!(
            BACnetSuccessFilter::from_legacy_bool(true),
            BACnetSuccessFilter::SUCCESSES_ONLY
        );
        assert_eq!(
            BACnetSuccessFilter::from_legacy_bool(false),
            BACnetSuccessFilter::ALL
        );
    }
}
