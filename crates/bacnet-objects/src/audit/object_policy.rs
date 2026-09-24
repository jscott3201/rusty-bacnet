//! Optional instance-owned overrides for the supported AV/BV Audit profile.
use crate::{
    common,
    property_metadata::{
        PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
    },
};
use bacnet_types::{
    bitstring::{AuditOperationFlags, BACnetPriorityFilter},
    enums::{AuditLevel, AuditOperation, ErrorClass, ErrorCode, PropertyIdentifier as P},
    error::Error,
    primitives::PropertyValue,
};

/// Value of a present Audit_Priority_Filter property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditPriorityPolicy {
    /// BACnet NULL: inherit the associated Reporter's command-priority filter.
    Inherit,
    /// An explicit sixteen-bit filter, including the empty filter.
    Filter(BACnetPriorityFilter),
}

/// Independently optional Audit properties owned by an Analog Value or Binary Value.
///
/// Provisioning any field opts that instance into the implemented Audit property
/// subset. Absent fields inherit the associated Reporter. DEFAULT level and a
/// present NULL priority filter also inherit. AV/BV follow their object-specific
/// clauses for absent priority inheritance; generic §19.6.3 has conflicting wording.
/// Both supported objects have commandable Present_Value; these properties do not
/// install a Reporter or enable reporting by themselves.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ObjectAuditPolicy {
    /// Absent row or the instance level; DEFAULT inherits.
    pub level: Option<AuditLevel>,
    /// Absent row or the instance operation selection, including an empty set.
    pub operations: Option<AuditOperationFlags>,
    /// Absent row, present NULL inheritance, or an explicit filter.
    pub priority_filter: Option<AuditPriorityPolicy>,
}

impl ObjectAuditPolicy {
    pub(crate) fn read(
        &self,
        property: P,
        index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let value = match property {
            P::AUDIT_LEVEL => self.level.map(|v| PropertyValue::Enumerated(v.to_raw())),
            P::AUDITABLE_OPERATIONS => self.operations.map(|v| bits(v.to_bacnet())),
            P::AUDIT_PRIORITY_FILTER => self.priority_filter.map(|v| match v {
                AuditPriorityPolicy::Inherit => PropertyValue::Null,
                AuditPriorityPolicy::Filter(filter) => bits(filter.to_bacnet()),
            }),
            _ => return None,
        };
        Some(
            value
                .ok_or_else(common::unknown_property_error)
                .and_then(|value| {
                    if index.is_some() {
                        Err(common::property_is_not_an_array_error())
                    } else {
                        Ok(value)
                    }
                }),
        )
    }

    pub(crate) fn write(
        &mut self,
        property: P,
        index: Option<u32>,
        value: &PropertyValue,
        priority: Option<u8>,
    ) -> Option<Result<(), Error>> {
        self.read(property, index).map(|present| {
            present?;
            if priority.is_some_and(|v| !(1..=16).contains(&v)) {
                return Err(Error::Protocol {
                    class: ErrorClass::SERVICES.to_raw() as u32,
                    code: ErrorCode::PARAMETER_OUT_OF_RANGE.to_raw() as u32,
                });
            }
            // NULL relinquishes ordinary noncommandable properties without a
            // change. For the nullable priority filter it is the actual value.
            if matches!(value, PropertyValue::Null) && property != P::AUDIT_PRIORITY_FILTER {
                return Ok(());
            }
            match (property, value) {
                (P::AUDIT_LEVEL, PropertyValue::Enumerated(v)) => {
                    self.level = Some(AuditLevel::from_raw(*v))
                }
                (P::AUDITABLE_OPERATIONS, PropertyValue::BitString { unused_bits, data }) => {
                    self.operations = Some(
                        AuditOperationFlags::from_bacnet(*unused_bits, data)
                            .map_err(|_| common::value_out_of_range_error())?,
                    );
                }
                (P::AUDIT_PRIORITY_FILTER, PropertyValue::Null) => {
                    self.priority_filter = Some(AuditPriorityPolicy::Inherit)
                }
                (P::AUDIT_PRIORITY_FILTER, PropertyValue::BitString { unused_bits, data }) => {
                    self.priority_filter = Some(AuditPriorityPolicy::Filter(
                        BACnetPriorityFilter::from_bacnet(*unused_bits, data)
                            .map_err(|_| common::value_out_of_range_error())?,
                    ));
                }
                _ => return Err(common::invalid_data_type_error()),
            }
            Ok(())
        })
    }

    pub(crate) fn metadata(&self) -> impl Iterator<Item = PropertyMetadata> {
        [
            (P::AUDIT_LEVEL, self.level.is_some()),
            (P::AUDITABLE_OPERATIONS, self.operations.is_some()),
            (P::AUDIT_PRIORITY_FILTER, self.priority_filter.is_some()),
        ]
        .into_iter()
        .filter(|(_, present)| *present)
        .map(|(property, _)| {
            PropertyMetadata::new(
                property,
                PropertyConformance::Optional,
                Some(if property == P::AUDIT_PRIORITY_FILTER {
                    PropertyPresenceCondition::CommandableAuditReporting
                } else {
                    PropertyPresenceCondition::ObjectAuditReporting
                }),
                PropertyWriteCapability::Always,
            )
        })
    }

    /// Resolve this AV/BV instance's settings against its associated Reporter.
    #[doc(hidden)]
    pub fn effective_internal(self, reporter: &super::AuditReporterObject) -> EffectiveAuditPolicy {
        EffectiveAuditPolicy {
            reporter_enabled: reporter.audit_level != AuditLevel::NONE,
            level: self
                .level
                .filter(|v| *v != AuditLevel::DEFAULT)
                .unwrap_or(reporter.audit_level),
            operations: self.operations.unwrap_or(reporter.auditable_operations),
            priorities: match self.priority_filter {
                Some(AuditPriorityPolicy::Filter(value)) => value,
                _ => reporter.audit_priority_filter,
            },
        }
    }
}

fn bits((unused_bits, data): (u8, Vec<u8>)) -> PropertyValue {
    PropertyValue::BitString { unused_bits, data }
}

/// Immutable per-operation policy snapshot; source reporting does not use it.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct EffectiveAuditPolicy {
    pub reporter_enabled: bool,
    pub level: AuditLevel,
    pub operations: AuditOperationFlags,
    pub priorities: BACnetPriorityFilter,
}
impl EffectiveAuditPolicy {
    pub fn reports(
        self,
        operation: AuditOperation,
        property: Option<P>,
        priority: Option<u8>,
    ) -> bool {
        self.reporter_enabled
            && self.level != AuditLevel::NONE
            && self.operations.contains(operation)
            && !(self.level == AuditLevel::AUDIT_CONFIG && property == Some(P::PRESENT_VALUE))
            && priority.is_none_or(|v| self.priorities.contains(v))
    }
}

#[cfg(test)]
#[path = "object_policy_tests.rs"]
mod tests;
