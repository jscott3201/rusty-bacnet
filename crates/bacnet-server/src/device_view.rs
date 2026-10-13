//! Executor-owned Device reads, independent of mutable object declarations.
//! This request-scoped adapter never changes the database or service dispatch.
use crate::cov::active::LiveDeviceCov;
use bacnet_objects::{
    audit::{AuditLogForwarding, AuditLogStorage, AuditReporterObject, ObjectAuditPolicy},
    database::ObjectDatabase,
    device::EXECUTED_SERVICES,
    event::EnrollmentSummaryCapability,
    event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource},
    file::{FileConfiguration, FileStorage},
    log_buffer::{LogBufferRecords, LogRecordIdentity},
    log_reporting::BufferReadyReport,
    property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability},
    schedule::ScheduleWrite,
    traits::{BACnetObject, CovReportedProperty},
};
use bacnet_types::{
    calendar::SpecificDate,
    constructed::BACnetObjectPropertyReference,
    enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, ServiceSupported},
    error::Error,
    primitives::{ObjectIdentifier, PropertyValue},
};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy)]
pub(crate) enum DeviceExecution {
    FullServer,
    Endpoint { writes: bool, reinitialize: bool },
}

impl DeviceExecution {
    pub(crate) fn services(self, clock: bool) -> impl Iterator<Item = ServiceSupported> {
        let services = match self {
            Self::FullServer => EXECUTED_SERVICES.to_vec(),
            Self::Endpoint {
                writes,
                reinitialize,
            } => {
                let mut services = vec![ServiceSupported::READ_PROPERTY];
                if writes {
                    services.push(ServiceSupported::WRITE_PROPERTY);
                }
                if reinitialize {
                    services.push(ServiceSupported::REINITIALIZE_DEVICE);
                }
                services
            }
        };
        services.into_iter().filter(move |service| {
            clock
                || !matches!(
                    *service,
                    ServiceSupported::TIME_SYNCHRONIZATION
                        | ServiceSupported::UTC_TIME_SYNCHRONIZATION
                )
        })
    }
}

/// One execution profile and clock observation, shared across a served request.
pub(crate) struct DeviceReadContext<'a> {
    pub(crate) registered_port: Option<ObjectIdentifier>,
    /// Result rows a single-property read of a Group's Present_Value may
    /// expand, its own row and the member rows (#1172). ReadPropertyMultiple
    /// charges its own budget instead; this starts at that budget's default,
    /// and the endpoint responder sets its configured limit (#1215).
    pub(crate) work_limit: usize,
    execution: DeviceExecution,
    clock: bool,
    live: Option<&'a LiveDeviceCov>,
}

impl<'a> DeviceReadContext<'a> {
    /// A context serving no live COV list; a request that reads one adds its
    /// snapshot with [`with_live`](Self::with_live) once its plan is known.
    pub(crate) fn new(db: &ObjectDatabase, execution: DeviceExecution) -> Self {
        Self {
            registered_port: None,
            work_limit: crate::server::ReadPropertyMultipleBudget::default().max_result_elements,
            execution,
            clock: db.clock_frame().is_some(),
            live: None,
        }
    }
    /// The same context, serving the selected Device's COV lists from `live`.
    /// A request plans with the context alone, since planning reads no value,
    /// then samples the lists its plan reads and serves them from here (#1213).
    pub(crate) fn with_live<'b>(self, live: Option<&'b LiveDeviceCov>) -> DeviceReadContext<'b> {
        DeviceReadContext {
            registered_port: self.registered_port,
            work_limit: self.work_limit,
            execution: self.execution,
            clock: self.clock,
            live,
        }
    }
    pub(crate) fn with_registered_port(mut self, oid: Option<ObjectIdentifier>) -> Self {
        self.registered_port = oid;
        self
    }
    pub(crate) fn with_work_limit(mut self, limit: usize) -> Self {
        self.work_limit = limit;
        self
    }
    pub(crate) fn object<'b>(&'b self, object: &'b dyn BACnetObject) -> DeviceReadView<'b> {
        DeviceReadView {
            object,
            context: self,
        }
    }
}

pub(crate) struct DeviceReadView<'a> {
    object: &'a dyn BACnetObject,
    context: &'a DeviceReadContext<'a>,
}

fn owned(property: P) -> bool {
    matches!(
        property,
        P::PROTOCOL_SERVICES_SUPPORTED
            | P::PROPERTY_LIST
            | P::ACTIVE_COV_SUBSCRIPTIONS
            | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
            | P::DEVICE_ADDRESS_BINDING
    )
}
/// The five served fields cannot be assigned through a custom Device writer.
/// Call only after the existing object/index/value and authorization preflight.
pub(crate) fn check_executor_owned_write(oid: ObjectIdentifier, property: P) -> Result<(), Error> {
    if oid.object_type() == ObjectType::DEVICE && owned(property) {
        Err(property_error(ErrorCode::WRITE_ACCESS_DENIED))
    } else {
        Ok(())
    }
}

fn property_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

impl DeviceReadView<'_> {
    fn is_device(&self) -> bool {
        self.object.object_identifier().object_type() == ObjectType::DEVICE
    }
    fn cov_present(&self) -> bool {
        matches!(self.context.execution, DeviceExecution::FullServer)
    }
}

impl BACnetObject for DeviceReadView<'_> {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.object.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.object.object_name()
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        if !self.is_device() || !owned(property) {
            return self.object.read_property(property, index);
        }
        if index.is_some() && !self.is_array_property(property) {
            return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
        }
        match property {
            P::PROPERTY_LIST => {
                let properties: Vec<_> = self
                    .property_list()
                    .iter()
                    .copied()
                    .filter(|property| {
                        !matches!(
                            *property,
                            P::OBJECT_IDENTIFIER
                                | P::OBJECT_NAME
                                | P::OBJECT_TYPE
                                | P::PROPERTY_LIST
                        )
                    })
                    .collect();
                match index {
                    None => Ok(PropertyValue::List(
                        properties
                            .iter()
                            .map(|property| PropertyValue::Enumerated(property.to_raw()))
                            .collect(),
                    )),
                    Some(0) => Ok(PropertyValue::Unsigned(properties.len() as u64)),
                    Some(index) => properties
                        .get((index - 1) as usize)
                        .map(|property| PropertyValue::Enumerated(property.to_raw()))
                        .ok_or_else(|| property_error(ErrorCode::INVALID_ARRAY_INDEX)),
                }
            }
            P::PROTOCOL_SERVICES_SUPPORTED => {
                // Clause 21 BACnetServicesSupported: 49 defined bits, MSB first.
                let mut data = vec![0; 7];
                for service in self.context.execution.services(self.context.clock) {
                    let bit = usize::from(service.to_raw());
                    data[bit / 8] |= 0x80 >> (bit % 8);
                }
                Ok(PropertyValue::BitString {
                    unused_bits: 7,
                    data,
                })
            }
            // The server's bindings, as the request sampled them for the
            // selected Device (#1369). Any other Device, and an endpoint,
            // which keeps no bindings to serve, read an empty list.
            P::DEVICE_ADDRESS_BINDING => Ok(self
                .context
                .live
                .and_then(|live| live.resolve(self.object_identifier(), property))
                .unwrap_or_else(|| PropertyValue::List(Vec::new()))),
            _ if self.cov_present() => Ok(self
                .context
                .live
                .and_then(|live| live.resolve(self.object_identifier(), property))
                .unwrap_or_else(|| PropertyValue::ApplicationData(Vec::new()))),
            _ => Err(property_error(ErrorCode::UNKNOWN_PROPERTY)),
        }
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(property_error(ErrorCode::WRITE_ACCESS_DENIED))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        if !self.is_device() {
            return self.object.property_metadata();
        }
        let original = self.object.property_metadata();
        let mut rows: Vec<_> = if original.is_empty() {
            let required = self.object.required_properties();
            self.object
                .property_list()
                .iter()
                .copied()
                .map(|property| {
                    PropertyMetadata::new(
                        property,
                        if required.contains(&property) {
                            PropertyConformance::RequiredRead
                        } else {
                            PropertyConformance::Optional
                        },
                        None,
                        if self.object.is_writable_property(property) {
                            PropertyWriteCapability::Always
                        } else {
                            PropertyWriteCapability::ReadOnly
                        },
                    )
                })
                .collect()
        } else {
            original.into_owned()
        };
        rows.retain(|row| !owned(row.property_identifier));
        for property in [
            P::PROTOCOL_SERVICES_SUPPORTED,
            P::PROPERTY_LIST,
            P::DEVICE_ADDRESS_BINDING,
        ] {
            rows.push(PropertyMetadata::new(
                property,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ));
        }
        if self.cov_present() {
            for property in [
                P::ACTIVE_COV_SUBSCRIPTIONS,
                P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
            ] {
                rows.push(PropertyMetadata::new(
                    property,
                    PropertyConformance::Optional,
                    None,
                    PropertyWriteCapability::ReadOnly,
                ));
            }
        }
        rows.sort_by_key(|row| row.property_identifier.to_raw());
        Cow::Owned(rows)
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        if !self.is_device() {
            return self.object.property_list();
        }
        Cow::Owned(
            self.property_metadata()
                .iter()
                .map(|row| row.property_identifier)
                .collect(),
        )
    }
    fn required_properties(&self) -> Cow<'static, [P]> {
        if !self.is_device() {
            return self.object.required_properties();
        }
        bacnet_objects::property_metadata::required_properties_from_metadata(
            &self.property_metadata(),
        )
    }
    fn is_array_property(&self, property: P) -> bool {
        if self.is_device() && owned(property) {
            property == P::PROPERTY_LIST
        } else {
            self.object.is_array_property(property)
        }
    }
    /// ReadRange asks this of the served object (#1046): the two COV lists
    /// and Device_Address_Binding are BACnetLISTs, Property_List is an array
    /// and the services bit string is a single value.
    fn is_list_property(&self, property: P) -> bool {
        if self.is_device() && owned(property) {
            matches!(
                property,
                P::ACTIVE_COV_SUBSCRIPTIONS
                    | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
                    | P::DEVICE_ADDRESS_BINDING
            )
        } else {
            self.object.is_list_property(property)
        }
    }
    fn is_writable_property(&self, property: P) -> bool {
        if self.is_device() && owned(property) {
            false
        } else {
            self.object.is_writable_property(property)
        }
    }
    /// A frozen copy can't carry this request's executor-owned values, so a
    /// Device offers none and a caller reads the view itself.
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        if self.is_device() {
            None
        } else {
            self.object.cov_snapshot_internal()
        }
    }

    // Every other read-only query is the wrapped object's own answer (#1076);
    // `tests::the_view_forwards_every_read_query` runs each query the trait
    // declares and fails on one this list lacks. The mutating hooks keep their
    // trait defaults: the view borrows the object shared and serves reads only.
    fn audit_object_policy_internal(&self) -> ObjectAuditPolicy {
        self.object.audit_object_policy_internal()
    }
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        self.object.audit_reporter_internal()
    }
    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.object.next_monotonic_deadline_internal()
    }
    fn lighting_blink_count_internal(&self) -> u64 {
        self.object.lighting_blink_count_internal()
    }
    fn is_createable(&self) -> bool {
        self.object.is_createable()
    }
    fn creation_only_properties(&self) -> &'static [P] {
        self.object.creation_only_properties()
    }
    fn is_deleteable(&self) -> bool {
        self.object.is_deleteable()
    }
    fn supports_cov(&self) -> bool {
        self.object.supports_cov()
    }
    fn supports_subscribe_cov_property(&self) -> bool {
        self.object.supports_subscribe_cov_property()
    }
    fn staging_generation_internal(&self) -> Option<u64> {
        self.object.staging_generation_internal()
    }
    fn command_generation_internal(&self) -> Option<u64> {
        self.object.command_generation_internal()
    }
    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        self.object.enrollment_summary_capability_internal()
    }
    fn supports_cov_property(&self, property: P) -> bool {
        self.object.supports_cov_property(property)
    }
    fn cov_increment(&self) -> Option<f64> {
        self.object.cov_increment()
    }
    fn cov_reported_properties(&self) -> &'static [CovReportedProperty] {
        self.object.cov_reported_properties()
    }
    fn calendar_state_internal(&self, day: SpecificDate) -> Option<bool> {
        self.object.calendar_state_internal(day)
    }
    fn retry_refusals_naming(&self, target: ObjectIdentifier) -> Option<ScheduleWrite> {
        self.object.retry_refusals_naming(target)
    }
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        self.object.enrollment_eval_state_internal()
    }
    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        self.object.enrollment_eval_source_internal()
    }
    fn input_reference_internal(&self) -> Option<Option<&BACnetObjectPropertyReference>> {
        self.object.input_reference_internal()
    }
    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.object.reliability_evaluation_inhibited_internal()
    }
    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        self.object.audit_log_storage_internal()
    }
    fn audit_log_forwarding_internal(&self) -> Option<Arc<AuditLogForwarding>> {
        self.object.audit_log_forwarding_internal()
    }
    fn file_configuration_internal(&self) -> Option<&dyn FileConfiguration> {
        self.object.file_configuration_internal()
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        self.object.file_storage_internal()
    }
    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        self.object.log_record_identities_internal()
    }
    fn log_buffer_internal(&self) -> Option<&dyn LogBufferRecords> {
        self.object.log_buffer_internal()
    }
    fn logs_received_event_notifications_internal(&self) -> bool {
        self.object.logs_received_event_notifications_internal()
    }
    fn buffer_ready_report_internal(&self) -> Option<BufferReadyReport> {
        self.object.buffer_ready_report_internal()
    }
    fn event_algorithm_inhibit_reference_internal(&self) -> Option<BACnetObjectPropertyReference> {
        self.object.event_algorithm_inhibit_reference_internal()
    }
}

#[cfg(test)]
#[path = "device_view_tests.rs"]
mod tests;
