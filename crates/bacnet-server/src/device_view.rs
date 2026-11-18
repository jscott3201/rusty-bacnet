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

/// What a full client executes beside a full server on one node (#2617):
/// I-Am and I-Have, which its discovery takes in, and the COV
/// notifications of its subscriptions, of one object or of several
/// (#2654), whose confirmed forms it acknowledges. The server already lists
/// both event notifications, whose confirmed form it acknowledges.
pub(crate) const NODE_CLIENT_SERVICES: &[ServiceSupported] = &[
    ServiceSupported::I_AM,
    ServiceSupported::I_HAVE,
    ServiceSupported::CONFIRMED_COV_NOTIFICATION,
    ServiceSupported::UNCONFIRMED_COV_NOTIFICATION,
    ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
    ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
];

/// What the Device's Protocol_Services_Supported lists, fixed when the
/// server starts and carried by every read (two octets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceExecution {
    /// A full server. With `time_sync` it executes TimeSynchronization and
    /// UTCTimeSynchronization once a Device clock is installed: its policy
    /// [executes](crate::server::TimeSyncPolicy::executes) them (#2617).
    FullServer { time_sync: bool },
    /// A full server on a node's seat beside the node's client role, which
    /// executes [`NODE_CLIENT_SERVICES`] too: the Device lists what the
    /// whole node executes (Clause 12.11.14). `time_sync` as for
    /// [`FullServer`](Self::FullServer).
    Node { time_sync: bool },
}

const _: () = assert!(std::mem::size_of::<DeviceExecution>() == 2);

/// The Device's COV subscription lists every profile serves (Table 12-13,
/// notes 9 and 18): the server runs SubscribeCOVPropertyMultiple too.
const COV_LISTS: &[P] = &[
    P::ACTIVE_COV_SUBSCRIPTIONS,
    P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
];

impl DeviceExecution {
    /// A full server's profile, beside a node's client role when `node`,
    /// executing time synchronization as `policy` does.
    pub(crate) fn server(node: bool, policy: &crate::server::TimeSyncPolicy) -> Self {
        let time_sync = policy.executes();
        if node {
            Self::Node { time_sync }
        } else {
            Self::FullServer { time_sync }
        }
    }

    /// Whether the profile executes both time-synchronization services
    /// once a Device clock is installed.
    fn time_sync(self) -> bool {
        match self {
            Self::FullServer { time_sync } | Self::Node { time_sync } => time_sync,
        }
    }

    /// What the profile executes, with both time-synchronization services
    /// only when it executes them and `clock`, a Device clock, is installed.
    pub(crate) fn services(self, clock: bool) -> impl Iterator<Item = ServiceSupported> {
        let clock = clock && self.time_sync();
        let services = match self {
            Self::FullServer { .. } => EXECUTED_SERVICES.to_vec(),
            Self::Node { .. } => EXECUTED_SERVICES
                .iter()
                .chain(NODE_CLIENT_SERVICES)
                .copied()
                .collect(),
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
    /// Only a server's time-synchronization bits depend on clock
    /// availability, and only while it executes those services. Cache the
    /// clock answer for this request; Device time values and metadata stay
    /// live.
    fn clock_available(&self) -> bool {
        self.execution.time_sync() && *self.clock.get_or_init(|| self.db.clock_frame().is_some())
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
        COV_LISTS.contains(&property)
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
            // local Device, the identity's (#1369, #2227). Any other Device
            // reads an empty list.
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
        for &property in COV_LISTS {
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
