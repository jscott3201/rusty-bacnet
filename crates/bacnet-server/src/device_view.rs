//! Executor-owned Device reads, independent of mutable object declarations.
//! This request-scoped adapter never changes the database or service dispatch.
#[path = "device_view_identity.rs"]
mod identity;
use bacnet_device_core::cov::active::LiveDeviceCov;
use bacnet_device_core::read::ReadOverlay;
use bacnet_device_core::write::{identity_property, owned};
use bacnet_objects::property_metadata::is_array_property;
use bacnet_objects::{
    audit::AuditReporterObject,
    database::ObjectDatabase,
    device::EXECUTED_SERVICES,
    file::{FileConfiguration, FileStorage},
    property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability},
    traits::{
        AuditLog, AuditPolicy, BACnetObject, CalendarState, CommandRunner, CovReporting,
        EnrollmentEval, EventReporting, LogBuffer,
    },
};
use bacnet_types::{
    enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, ServiceSupported},
    error::Error,
    primitives::{ObjectIdentifier, PropertyValue},
};
use identity::IdentityView;
use std::borrow::Cow;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(crate) enum DeviceExecution {
    FullServer,
    /// An endpoint's responder: ReadProperty, and each service its session
    /// enabled. `cov` is SubscribeCOV, SubscribeCOVProperty and
    /// SubscribeCOVPropertyMultiple (#2490).
    Endpoint {
        writes: bool,
        reinitialize: bool,
        cov: bool,
    },
}

impl DeviceExecution {
    pub(crate) fn services(self, clock: bool) -> impl Iterator<Item = ServiceSupported> {
        let services = match self {
            Self::FullServer => EXECUTED_SERVICES.to_vec(),
            Self::Endpoint {
                writes,
                reinitialize,
                cov,
            } => {
                let mut services = vec![ServiceSupported::READ_PROPERTY];
                if writes {
                    services.push(ServiceSupported::WRITE_PROPERTY);
                }
                if reinitialize {
                    services.push(ServiceSupported::REINITIALIZE_DEVICE);
                }
                if cov {
                    services.push(ServiceSupported::SUBSCRIBE_COV);
                    services.push(ServiceSupported::SUBSCRIBE_COV_PROPERTY);
                    services.push(ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE);
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

    /// The Device's COV subscription lists this execution serves: both for
    /// the full server and for an endpoint with COV on, which runs
    /// SubscribeCOVPropertyMultiple too (Table 12-13, notes 9 and 18);
    /// neither for an endpoint without it.
    fn cov_lists(self) -> &'static [P] {
        match self {
            Self::FullServer | Self::Endpoint { cov: true, .. } => &[
                P::ACTIVE_COV_SUBSCRIPTIONS,
                P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
            ],
            Self::Endpoint { cov: false, .. } => &[],
        }
    }
}

/// One execution profile and lazy clock availability, shared across a served
/// request: the server's [`ReadOverlay`]. The request's work limit and
/// wildcard targets are its [`ReadScope`](bacnet_device_core::read::ReadScope).
pub(crate) struct DeviceReadContext<'a> {
    execution: DeviceExecution,
    db: &'a ObjectDatabase,
    clock: OnceLock<bool>,
    live: Option<&'a LiveDeviceCov>,
    identity: Option<IdentityView>,
}

impl<'a> DeviceReadContext<'a> {
    /// A context serving no live COV list; a request that reads one adds its
    /// snapshot with [`with_live`](Self::with_live) once its plan is known.
    pub(crate) fn new(db: &'a ObjectDatabase, execution: DeviceExecution) -> Self {
        Self {
            execution,
            db,
            clock: OnceLock::new(),
            live: None,
            identity: None,
        }
    }
    /// The same context, serving the local Device's COV lists from `live`.
    /// A request plans with the context alone, since planning reads no value,
    /// then samples the lists its plan reads and serves them from here (#1213).
    pub(crate) fn with_live<'b>(self, live: Option<&'b LiveDeviceCov>) -> DeviceReadContext<'b>
    where
        'a: 'b,
    {
        DeviceReadContext {
            execution: self.execution,
            db: self.db,
            clock: self.clock,
            live,
            identity: self.identity,
        }
    }
    /// Only full-server service bits depend on clock availability. Cache both
    /// answers for this request; Device time values and metadata stay live.
    fn clock_available(&self) -> bool {
        matches!(self.execution, DeviceExecution::FullServer)
            && *self.clock.get_or_init(|| self.db.clock_frame().is_some())
    }
    pub(crate) fn with_identity(
        mut self,
        identity: Option<&bacnet_objects::DeviceIdentity>,
    ) -> Self {
        self.identity = identity.map(IdentityView::from);
        self
    }
    /// The executor's fixed wildcard target, even when its object is absent.
    pub(crate) fn device_oid(&self) -> Option<ObjectIdentifier> {
        self.identity.map(|identity| identity.oid)
    }
    pub(crate) fn object<'b>(&'b self, object: &'b dyn BACnetObject) -> DeviceReadView<'b> {
        DeviceReadView {
            object,
            context: self,
        }
    }
}

impl<'c> ReadOverlay for DeviceReadContext<'c> {
    type View<'a>
        = DeviceReadView<'a>
    where
        Self: 'a;

    fn view<'a>(&'a self, stored: &'a dyn BACnetObject) -> Option<DeviceReadView<'a>> {
        Some(self.object(stored))
    }
}

pub(crate) struct DeviceReadView<'a> {
    object: &'a dyn BACnetObject,
    context: &'a DeviceReadContext<'a>,
}

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

impl DeviceReadView<'_> {
    fn identity(&self) -> Option<&IdentityView> {
        self.context
            .identity
            .as_ref()
            .filter(|identity| identity.oid == self.object.object_identifier())
    }
    fn owns(&self, property: P) -> bool {
        owned(property) || (self.identity().is_some() && identity_property(property))
    }
    fn is_device(&self) -> bool {
        self.object.object_identifier().object_type() == ObjectType::DEVICE
    }
    fn cov_present(&self, property: P) -> bool {
        self.context.execution.cov_lists().contains(&property)
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
        if let Some(identity) = self.identity().filter(|_| identity_property(property)) {
            let value = identity.value(property)?;
            return if index.is_some() {
                Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
            } else {
                Ok(value)
            };
        }
        if !self.is_device() || !owned(property) {
            return self.object.read_property(property, index);
        }
        if index.is_some() && !is_array_property(self, property) {
            return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
        }
        match property {
            P::PROPERTY_LIST => bacnet_objects::property_metadata::read_property_list(
                &self.property_metadata(),
                index,
            ),
            P::PROTOCOL_SERVICES_SUPPORTED => {
                // Clause 21 BACnetServicesSupported: 49 defined bits, MSB first.
                let mut data = vec![0; 7];
                for service in self
                    .context
                    .execution
                    .services(self.context.clock_available())
                {
                    let bit = usize::from(service.to_raw());
                    data[bit / 8] |= 0x80 >> (bit % 8);
                }
                Ok(PropertyValue::BitString {
                    unused_bits: 7,
                    data,
                })
            }
            // The server's bindings, as the request sampled them for the
            // local Device, the identity's (#1369, #2227). Any other Device, and an endpoint,
            // which keeps no bindings to serve, read an empty list.
            P::DEVICE_ADDRESS_BINDING => Ok(self
                .context
                .live
                .and_then(|live| live.resolve(self.object_identifier(), property))
                .unwrap_or_else(|| PropertyValue::List(Vec::new()))),
            _ if self.cov_present(property) => Ok(self
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
        let mut rows = self.object.property_metadata().into_owned();
        rows.retain(|row| !self.owns(row.property_identifier));
        if let Some(identity) = self.identity() {
            rows.extend(identity.properties().map(|property| {
                PropertyMetadata::new(
                    property,
                    PropertyConformance::RequiredRead,
                    None,
                    PropertyWriteCapability::ReadOnly,
                )
            }));
        }
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
        for &property in self.context.execution.cov_lists() {
            rows.push(PropertyMetadata::new(
                property,
                PropertyConformance::Optional,
                None,
                PropertyWriteCapability::ReadOnly,
            ));
        }
        rows.sort_by_key(|row| row.property_identifier.to_raw());
        Cow::Owned(rows)
    }
    // Every other read-only query is the wrapped object's own answer (#1076);
    // `tests::the_view_forwards_every_read_query` runs each query the trait
    // declares and fails on one this list lacks. The mutating hooks keep their
    // trait defaults: the view borrows the object shared and serves reads only.
    fn audit_policy(&self) -> Option<&dyn AuditPolicy> {
        self.object.audit_policy()
    }
    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        self.object.audit_reporter()
    }
    fn command_runner(&self) -> Option<&dyn CommandRunner> {
        self.object.command_runner()
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.object.event_reporting()
    }
    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        self.object.enrollment_eval()
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        self.object.cov()
    }
    fn calendar_state(&self) -> Option<&dyn CalendarState> {
        self.object.calendar_state()
    }
    fn audit_log(&self) -> Option<&dyn AuditLog> {
        self.object.audit_log()
    }
    fn file_configuration(&self) -> Option<&dyn FileConfiguration> {
        self.object.file_configuration()
    }
    fn file_storage(&self) -> Option<&dyn FileStorage> {
        self.object.file_storage()
    }
    fn log_buffer(&self) -> Option<&dyn LogBuffer> {
        self.object.log_buffer()
    }
}

#[cfg(test)]
#[path = "device_view_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "device_view_clock_tests.rs"]
mod clock_tests;
