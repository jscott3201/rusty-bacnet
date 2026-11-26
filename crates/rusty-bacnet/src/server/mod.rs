//! Python BACnetServer — async wrapper around the Rust BACnetServer.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use bacnet_objects::access_control::{
    AccessCredentialObject, AccessDoorObject, AccessPointObject, AccessRightsObject,
    AccessUserObject, AccessZoneObject, CredentialDataInputObject,
};
use bacnet_objects::accumulator::{AccumulatorObject, PulseConverterObject};
use bacnet_objects::audit::{AuditLogObject, AuditReporterObject, FileAuditLogPersistence};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::file::FileObject;
use bacnet_objects::group::{GlobalGroupObject, StructuredViewObject};
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::load_control::{FileLoadControlPersistence, LoadControlObject};
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::notification_class::{FileNotificationClassPersistence, NotificationClass};
use bacnet_objects::program::ProgramObject;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_objects::timer::TimerObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_objects::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};
use bacnet_server::server;

use crate::app_transport::AppTransport;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetStageLimitValue,
};
use bacnet_types::enums::{EventType, ObjectType};
use bacnet_types::primitives::PropertyValue;
use bacnet_types::sync::MutexExt;

use crate::errors::to_py_err;
use crate::point_registration::admit;
use crate::types::{PyEventType, PyObjectIdentifier, PyPropertyIdentifier, PyPropertyValue};

mod audit_configuration;
use audit_configuration::AuditNotificationSink;
mod host;
mod registered_port;
mod start_claim;
pub(crate) use audit_configuration::{
    reporter_audit_level, reporter_auditable_operations, validate_audit_recipient,
};
pub(crate) use host::{SeatedHost, ServerHost};

/// What a call only an unstarted server takes raises once a start has
/// begun (#2819): `BacnetStateError`, `state` `"already_started"`.
fn already_started(message: &str) -> PyErr {
    crate::errors::lifecycle_error(bacnet_types::error::Lifecycle::AlreadyStarted, message)
}

/// Async BACnet server that hosts objects and responds to requests.
///
/// Usage:
/// ```python
/// server = BACnetServer(device_instance=1234, device_name="My Device")
/// server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
/// await server.start()
/// # ... server is now responding to BACnet requests ...
/// await server.stop()
/// ```
///
/// Supports multiple transports via the `transport` parameter:
/// - `"bip"` (default): BACnet/IP over UDP
/// - `"ipv6"`: BACnet/IPv6 over UDP multicast. The server's VMAC is its
///   `device_instance` (`bip6:0x0004D2` for 1234); startup fails when another
///   node on the link already uses it. `foreign_device=` with a B/IPv6 BBMD
///   registers it as a foreign device (#1941).
/// - `"sc"`: BACnet/SC over TLS WebSocket (requires `sc_hub`, `sc_vmac`,
///   `sc_ca_cert`, `sc_client_cert`, `sc_client_key`, and persistent `sc_device_uuid`)
/// - `"mstp"`: BACnet MS/TP over RS-485 (requires `serial_port`)
/// - `"router"`: the server share of `router=`, a `BACnetRouter` built with
///   `application="client_and_server"` (#2071). The server answers at the
///   router's address on each of its networks.
///
/// SC credential paths must be nonempty at construction (ValueError otherwise).
/// start() loads the files before dialing or draining registrations; local TLS
/// configuration errors raise RuntimeError and permit retry after file repair.
/// Later startup failures retain existing behavior, not general rollback.
#[pyclass(name = "BACnetServer", module = "rusty_bacnet")]
pub struct BACnetServer {
    /// The running server. Every call takes a read guard, so a local write
    /// that waits for its event notifications (a Device recipient's I-Am,
    /// #2537) holds up no other call; `start` and `stop` take the write
    /// guard, `stop` once the calls in flight have returned. An endpoint's
    /// server takes its endpoint's session lock in the same way (#2617).
    inner: ServerHost,
    device_instance: u32,
    segmentation_supported: bacnet_types::enums::Segmentation,
    apdu_segment_timeout_ms: u64,
    device_name: String,
    transport_type: String,
    // BIP config
    interface: std::net::Ipv4Addr,
    port: u16,
    broadcast_address: String,
    /// See `BipTransport::set_share_port_by_address` (#1538).
    share_port_by_address: bool,
    /// See `BipTransport::set_share_wildcard_port` (#2366) and
    /// `Bip6Transport::set_share_wildcard_port` (#2385).
    share_wildcard_port: bool,
    /// The B/IP mode `bbmd=` or `foreign_device=` asked for (#938).
    bip_mode: bacnet_transport::bip::BipMode,
    /// The B/IPv6 foreign-device registration `foreign_device=` asked for
    /// (#1941).
    bip6_foreign: Option<bacnet_transport::bip6::Bip6ForeignDeviceConfig>,
    /// The B/IP multicast group `multicast=` asked for (#941).
    multicast: Option<bacnet_transport::bip::BipMulticast>,
    registered_network_port: Option<u32>,
    // SC config
    sc_hub: Option<String>,
    sc_vmac: Option<Vec<u8>>,
    sc_device_uuid: [u8; 16],
    sc_npdu_admission_policy: bacnet_transport::sc::ScNpduAdmissionPolicy,
    sc_ca_cert: Option<String>,
    sc_client_cert: Option<String>,
    sc_client_key: Option<String>,
    sc_heartbeat_interval_ms: Option<u64>,
    sc_heartbeat_timeout_ms: Option<u64>,
    // IPv6 config
    ipv6_interface: Option<String>,
    /// The MS/TP keywords, `serial_port` included.
    mstp: crate::mstp_py::MstpOptions,
    // Passwords
    dcc_password: Option<String>,
    mutation_policy: bacnet_server::mutation::MutationPolicy,
    /// The `mutation_policy` callable, when one was given (#1966).
    mutation_callback: Option<Arc<crate::mutation_policy::PyMutationPolicy>>,
    dcc_policy: server::DccPolicy,
    dcc_source_restriction: Option<server::DccSourceRestriction>,
    dcc_disable_rate_limit: Option<server::DccDisableRateLimit>,
    reinit_password: Option<String>,
    /// The `on_reinitialize` and `after_reinitialize` callables (#1566).
    reinitialize: Option<Arc<crate::reinitialize::PyReinitialize>>,
    request_admission_policy: server::RequestAdmissionPolicy,
    read_property_multiple_budget: bacnet_device_core::read::ReadPropertyMultipleBudget,
    get_alarm_summary_budget: server::GetAlarmSummaryBudget,
    get_enrollment_summary_budget: server::GetEnrollmentSummaryBudget,
    atomic_read_file_budget: server::AtomicReadFileBudget,
    atomic_write_file_budget: server::AtomicWriteFileBudget,
    read_range_budget: server::ReadRangeBudget,
    get_event_information_budget: bacnet_device_core::event::GetEventInformationBudget,
    cov_policy: bacnet_device_core::cov::CovPolicy,
    time_sync_policy: server::TimeSyncPolicy,
    /// The zone the Device clock starts in (`utc_offset_minutes`,
    /// `daylight_savings`).
    clock_zone: server::ClockConfig,
    /// Whether a peer may write the Device's UTC_Offset and
    /// Daylight_Savings_Status (`clock_zone_writable`, #2395).
    clock_zone_writable: bool,
    /// The file that keeps what peers write to the Device
    /// (`device_storage_path`, #2730).
    device_storage: Option<bacnet_objects::object_profile::FileTagsPersistence>,
    audit_notification_sink: Option<AuditNotificationSink>,
    audit_reporters: Option<server::AuditReportersConfig>,
    audit_recipient: Arc<std::sync::Mutex<Option<bacnet_types::constructed::BACnetRecipient>>>,
    device_bindings: std::collections::BTreeMap<u32, server::DeviceBinding>,
    /// Freeze forwarding and Reporter settings at ownership transfer, including startup in flight.
    forwarding_configuration_started: Arc<AtomicBool>,
    /// Whether a `start()` holds the registrations it took and has not
    /// finished: `add_*` and a second `start()` refuse meanwhile, so what
    /// a refused start gives back cannot clash with a later call.
    start_in_flight: Arc<AtomicBool>,
    /// Whether the server has been started.
    started: Arc<AtomicBool>,
    /// Objects to add before starting. A start takes them; one that is
    /// refused before it hands them to the database gives them back.
    pending_objects: Arc<std::sync::Mutex<Vec<Box<dyn BACnetObject + Send>>>>,
    /// Save counters of the pending Notification Forwarders, by instance,
    /// pushed with each forwarder under the `pending_objects` lock.
    pending_forwarder_save_counters: Arc<std::sync::Mutex<Vec<ForwarderSaveCounterEntry>>>,
    /// Save counters of the forwarders the running server holds, replaced
    /// under the `inner` lock when a start publishes its server.
    forwarder_save_counters: Arc<std::sync::Mutex<Vec<ForwarderSaveCounterEntry>>>,
    /// The router whose application entity `transport="router"` runs on
    /// (#2071).
    router: Option<crate::router::application::RouterApplication>,
}

/// One Notification Forwarder's instance and its shared save counters.
type ForwarderSaveCounterEntry = (
    u32,
    bacnet_objects::notification_forwarder::ForwarderSaveCounters,
);

impl BACnetServer {
    /// Lock the pending_objects mutex, poisoned or not (#2559): objects are
    /// pushed or drained in one step, and the in-place edits (file
    /// contents, Audit settings) go through the object's own validated
    /// setters, so a panic under the lock leaves whole objects.
    fn lock_pending(&self) -> std::sync::MutexGuard<'_, Vec<Box<dyn BACnetObject + Send>>> {
        self.pending_objects.locked()
    }

    /// Keep `object` for start, once [`admit`] has checked its identifier
    /// and name against the Device and the objects kept so far, under the
    /// same lock as the check that the server takes objects (#2748).
    /// Every `add_*` builder ends here, so each raises at the call: a
    /// ValueError for a repeated identifier or name, a BacnetError for an
    /// instance no object may have.
    fn push_pending(&self, object: Box<dyn BACnetObject + Send>) -> PyResult<()> {
        let name = object.object_name().to_string();
        let identifier = object.object_identifier();
        self.admit_pending(
            (
                identifier.object_type(),
                identifier.instance_number(),
                &name,
            ),
            || Ok(()),
            || Ok(object),
        )
    }

    /// [`admit`] one object into the pending list: refuse a call that comes
    /// too late, then run the checks of `admit` against the Device and
    /// every object kept, then `build` it and keep it. A builder with
    /// storage passes its own `check` and `build`, so a refusal comes
    /// before anything touches a file.
    fn admit_pending(
        &self,
        identity: (ObjectType, u32, &str),
        check: impl FnOnce() -> PyResult<()>,
        build: impl FnOnce() -> PyResult<Box<dyn BACnetObject + Send>>,
    ) -> PyResult<()> {
        self.admit_pending_with(identity, check, build, || Ok(()))
    }

    /// [`admit_pending`](Self::admit_pending), running `kept` once the
    /// object is admitted and built, before it joins the list and under
    /// the same lock: a refusal from it keeps nothing.
    fn admit_pending_with(
        &self,
        identity: (ObjectType, u32, &str),
        check: impl FnOnce() -> PyResult<()>,
        build: impl FnOnce() -> PyResult<Box<dyn BACnetObject + Send>>,
        kept: impl FnOnce() -> PyResult<()>,
    ) -> PyResult<()> {
        let mut pending = self.lock_pending();
        self.check_takes_objects()?;
        let taken = self.taken(&pending);
        let object = admit(identity, taken, check, build)?;
        kept()?;
        pending.push(object);
        Ok(())
    }

    /// Run the checks a builder with storage makes before it opens or
    /// clears a file (#2748): the same ones [`push_pending`](Self::push_pending)
    /// repeats under the lock when the object is kept.
    fn precheck_pending(&self, object_type: ObjectType, instance: u32, name: &str) -> PyResult<()> {
        let pending = self.lock_pending();
        self.check_takes_objects()?;
        let taken = self.taken(&pending);
        admit((object_type, instance, name), taken, || Ok(()), || Ok(()))
    }

    /// What a new object's identifier and name are checked against: the
    /// Device, then the objects kept so far.
    fn taken<'a>(
        &'a self,
        pending: &'a [Box<dyn BACnetObject + Send>],
    ) -> impl Iterator<Item = (ObjectType, u32, &'a str)> {
        let device = (
            ObjectType::DEVICE,
            self.device_instance,
            self.device_name.as_str(),
        );
        let kept = pending.iter().map(|object| {
            let identifier = object.object_identifier();
            (
                identifier.object_type(),
                identifier.instance_number(),
                object.object_name(),
            )
        });
        std::iter::once(device).chain(kept)
    }

    /// Refuse a change to the pending registrations while a `start()` holds
    /// them: what it gives back after a refusal must not meet a newer call.
    fn check_start_not_in_flight(&self) -> PyResult<()> {
        if self.start_in_flight.load(Ordering::Acquire) {
            return Err(already_started(
                "start() is in progress: registrations and their configuration \
                 cannot change until it ends",
            ));
        }
        Ok(())
    }

    /// Refuse an object once the server runs (call it under the pending
    /// lock). An endpoint's server (#2617) takes its objects from the
    /// endpoint only.
    fn check_takes_objects(&self) -> PyResult<()> {
        if self.inner.is_seated() {
            return Err(PyRuntimeError::new_err(
                "an endpoint's server takes its objects from the endpoint: add them with the \
                 endpoint's add_* calls before its start",
            ));
        }
        self.check_start_not_in_flight()?;
        if self.started.load(Ordering::Acquire) {
            return Err(already_started(
                "cannot add objects after start() — server is already running",
            ));
        }
        Ok(())
    }

    /// Read a member list given to an `add_*` method, a reference tuple or
    /// mapping per element. A member naming this server's own Device is
    /// stored in its local form, as the server stores the same member written
    /// over the network.
    fn members_from_py(
        &self,
        members: &Bound<'_, PyAny>,
        name: &str,
    ) -> PyResult<Vec<BACnetDeviceObjectPropertyReference>> {
        let mut members = crate::types::property_references_from_py(members, name)?;
        crate::types::localize(
            &mut members,
            crate::types::local_device(self.device_instance),
        );
        Ok(members)
    }
}

// The endpoints' `cov_policy` and `time_sync_policy` keywords are read as
// `BACnetServer`'s are (#2617).
pub(crate) use server_methods::{cov_policy, time_sync_policy};

mod server_methods {
    mod access_control_methods;
    mod access_credential_methods;
    mod access_input_methods;
    mod access_rights_methods;
    mod accumulator_methods;
    mod audit_log_methods;
    mod averaging_methods;
    mod binary_io_methods;
    mod channel_methods;
    mod clock_zone;
    mod constructor;
    mod constructor_budgets;
    mod cov_counters;
    pub(crate) mod cov_policy;
    mod dcc_outcomes;
    mod event_notification_counters;
    mod file_configuration;
    mod foreign_registration;
    mod life_safety_methods;
    mod lifecycle;
    mod lighting_methods;
    mod load_control_methods;
    mod loop_methods;
    mod network_port;
    mod notification_forwarder;
    mod policy_dict;
    mod program_methods;
    mod registration;
    mod reliability_methods;
    mod request_admission;
    mod sc_npdu;
    pub(crate) mod time_sync_policy;
    mod trend_log_methods;
    mod value_registration;
}
