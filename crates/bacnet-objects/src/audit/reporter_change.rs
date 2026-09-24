//! Object-owned configuration snapshots and the active target mutation boundary.
use super::*;
use crate::{clock::ClockFrame, device::AuditWriteSource};

/// One coherent Reporter property snapshot. Runtime readers never mutate it.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditReporterConfiguration {
    pub description: String,
    pub audit_level: AuditLevel,
    pub auditable_operations: AuditOperationFlags,
    pub confirmed: bool,
    pub monitored_objects: Option<Vec<BACnetObjectSelector>>,
    pub audit_priority_filter: BACnetPriorityFilter,
}
impl Default for AuditReporterConfiguration {
    fn default() -> Self {
        Self {
            description: String::new(),
            audit_level: AuditLevel::NONE,
            auditable_operations: AuditOperationFlags::empty(),
            confirmed: false,
            monitored_objects: None,
            audit_priority_filter: BACnetPriorityFilter::all(),
        }
    }
}
impl AuditReporterConfiguration {
    pub fn enabled(&self) -> bool {
        self.audit_level != AuditLevel::NONE
    }
    /// Nominal membership, independent of operation/value filters or self fallback.
    pub fn monitors(&self, target: ObjectIdentifier) -> bool {
        self.monitored_objects.as_ref().is_none_or(|selectors| {
            selectors.iter().any(|s| match s {
                BACnetObjectSelector::None => false,
                BACnetObjectSelector::Object(oid) => *oid == target,
                BACnetObjectSelector::ObjectType(kind) => *kind == target.object_type(),
            })
        })
    }
    pub fn monitors_unassigned(&self, kind: ObjectType) -> bool {
        self.monitored_objects.as_ref().is_none_or(|selectors| {
            selectors.iter().any(
                |s| matches!(s, BACnetObjectSelector::ObjectType(selected) if *selected == kind),
            )
        })
    }
    pub fn property(&self, property: PropertyIdentifier) -> Option<PropertyValue> {
        let bits = |(unused_bits, data)| PropertyValue::BitString { unused_bits, data };
        match property {
            PropertyIdentifier::DESCRIPTION => {
                Some(PropertyValue::CharacterString(self.description.clone()))
            }
            PropertyIdentifier::AUDIT_LEVEL => {
                Some(PropertyValue::Enumerated(self.audit_level.to_raw()))
            }
            PropertyIdentifier::AUDITABLE_OPERATIONS => {
                Some(bits(self.auditable_operations.to_bacnet()))
            }
            PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS => {
                Some(PropertyValue::Boolean(self.confirmed))
            }
            PropertyIdentifier::AUDIT_PRIORITY_FILTER => {
                Some(bits(self.audit_priority_filter.to_bacnet()))
            }
            PropertyIdentifier::MONITORED_OBJECTS => self.monitored_objects.as_ref().map(|v| {
                PropertyValue::List(
                    v.iter()
                        .map(BACnetObjectSelector::encode_property_value)
                        .collect(),
                )
            }),
            _ => None,
        }
    }
}

/// Weakly installed target owner. Prepare every notification before committing
/// the object-owned configuration, without awaiting or reacquiring the database.
#[doc(hidden)]
pub trait AuditReporterChangeSink: Send + Sync {
    fn is_active(&self) -> bool;
    fn change(
        &self,
        reporter: ObjectIdentifier,
        status: &Arc<AuditReporterStatus>,
        next: AuditReporterConfiguration,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error>;
}

/// Scoped concrete Reporter capability; it never exposes a replaceable object.
#[doc(hidden)]
pub struct AuditReporterAuthority<'a>(pub(super) &'a mut AuditReporterObject);
impl AuditReporterAuthority<'_> {
    pub fn object_identifier(&self) -> ObjectIdentifier {
        self.0.oid
    }
    pub fn validate_installation(&self) -> Result<(), Error> {
        if self
            .0
            .change_sink
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some()
            || self.0.change_owner.upgrade().is_some()
        {
            return Err(crate::common::write_access_denied_error());
        }
        Ok(())
    }
    pub fn install(
        &mut self,
        sink: &Arc<dyn AuditReporterChangeSink>,
        owner: &Arc<crate::database::AuditOwnership>,
    ) -> Result<(), Error> {
        self.validate_installation()?;
        self.0.change_sink = Some(Arc::downgrade(sink));
        self.0.change_owner = Arc::downgrade(owner);
        Ok(())
    }
    pub fn uninstall(&mut self, sink: &Arc<dyn AuditReporterChangeSink>) {
        if !sink.is_active()
            && self
                .0
                .change_sink
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, sink))
        {
            self.0.change_sink = None;
            self.0.change_owner = std::sync::Weak::new();
        }
    }
    pub fn write_description(
        &mut self,
        value: PropertyValue,
        index: Option<u32>,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        if index.is_some() {
            return Err(crate::common::property_is_not_an_array_error());
        }
        let mut next = self.0.configuration_internal();
        crate::common::write_description(
            &mut next.description,
            PropertyIdentifier::DESCRIPTION,
            &value,
        )
        .expect("Description branch")?;
        self.0.change_configuration(next, source)
    }
}

impl AuditReporterObject {
    pub(super) fn change_configuration(
        &mut self,
        next: AuditReporterConfiguration,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        if next.audit_level == AuditLevel::DEFAULT {
            return Err(Error::OutOfRange(
                "Audit Reporter audit level must not be DEFAULT".into(),
            ));
        }
        if let Some(sink) = self.change_sink.as_ref().and_then(std::sync::Weak::upgrade) {
            if !sink.is_active() {
                return Err(crate::common::write_access_denied_error());
            }
            if self.configuration_internal() == next {
                return Ok(());
            }
            return sink.change(
                self.oid,
                &self.status,
                next,
                source,
                self.clock.as_ref().and_then(|clock| clock.read_clock()),
            );
        }
        if self.change_owner.upgrade().is_some() {
            return Err(crate::common::write_access_denied_error());
        }
        self.status.commit_configuration(next, |_| Ok(()))
    }
    #[doc(hidden)]
    pub fn configuration_internal(&self) -> AuditReporterConfiguration {
        self.status.configuration()
    }
    #[doc(hidden)]
    pub fn captures_changes_internal(&self) -> bool {
        self.change_sink
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|sink| sink.is_active())
    }
}
