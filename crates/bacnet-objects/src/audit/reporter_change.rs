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
    pub maximum_send_delay: Option<AuditSendDelay>,
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
            maximum_send_delay: None,
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
            PropertyIdentifier::MAXIMUM_SEND_DELAY => self
                .maximum_send_delay
                .map(|delay| PropertyValue::Unsigned(u64::from(delay.seconds()))),
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
    fn send_now(
        &self,
        reporter: ObjectIdentifier,
        status: &Arc<AuditReporterStatus>,
        value: bool,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error>;
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
pub struct AuditReporterAuthority<'a>(
    pub(super) &'a mut AuditReporterObject,
    /// The source Audit owner [`configure`](Self::configure) answers to.
    Option<&'a alloc::sync::Weak<crate::database::AuditOwnership>>,
);
impl<'a> AuditReporterAuthority<'a> {
    pub(super) fn new(reporter: &'a mut AuditReporterObject) -> Self {
        Self(reporter, None)
    }
    /// The same authority answering to a source Audit owner: while `owner`
    /// is active, [`configure`](Self::configure) refuses Monitored_Objects
    /// and a send delay, which source Audit doesn't support. Only that
    /// check changes.
    pub fn with_source_owner(
        self,
        owner: &'a alloc::sync::Weak<crate::database::AuditOwnership>,
    ) -> Self {
        Self(self.0, Some(owner))
    }
}
impl AuditReporterAuthority<'_> {
    /// Atomically replace all trusted local Reporter settings without a
    /// network write.
    ///
    /// Invalid settings are refused before any mutation. `None` selectors
    /// remove Monitored_Objects (catch-all); `Some(vec![])` retains the
    /// property and selects no ordinary targets. An installed target owner
    /// prepares required records and lifecycle admission.
    ///
    /// Supply every field together, including the optional paired delay
    /// capability. Active target ownership fixes pair presence; an active
    /// source owner ([`with_source_owner`](Self::with_source_owner)) rejects
    /// it. `None` omits both delay/control properties; a present zero is
    /// immediate.
    pub fn configure(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
        maximum_send_delay: Option<AuditSendDelay>,
    ) -> Result<(), Error> {
        let source_active = self
            .1
            .is_some_and(|owner| owner.upgrade().is_some_and(|owner| owner.is_active()));
        if source_active && (selectors.is_some() || maximum_send_delay.is_some()) {
            return Err(Error::Encoding(
                "source Audit does not support Monitored_Objects or delayed notifications".into(),
            ));
        }
        let mut next = self.0.configuration_internal();
        next.audit_level = level;
        next.auditable_operations = operations;
        next.confirmed = confirmed;
        next.monitored_objects = selectors;
        next.audit_priority_filter = priorities;
        next.maximum_send_delay = maximum_send_delay;
        self.0.change_configuration(next, None)
    }
    pub fn object_identifier(&self) -> ObjectIdentifier {
        self.0.oid
    }
    pub fn validate_installation(&self) -> Result<(), Error> {
        if self
            .0
            .change_sink
            .as_ref()
            .and_then(alloc::sync::Weak::upgrade)
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
                .and_then(alloc::sync::Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, sink))
        {
            self.0.change_sink = None;
            self.0.change_owner = alloc::sync::Weak::new();
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
        if let Some(sink) = self
            .change_sink
            .as_ref()
            .and_then(alloc::sync::Weak::upgrade)
        {
            if !sink.is_active() {
                return Err(crate::common::write_access_denied_error());
            }
            if self.configuration_internal().maximum_send_delay.is_some()
                != next.maximum_send_delay.is_some()
            {
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
            .and_then(alloc::sync::Weak::upgrade)
            .is_some_and(|sink| sink.is_active())
    }
}
