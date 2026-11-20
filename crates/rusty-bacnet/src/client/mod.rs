//! Python BACnetClient — async wrapper around the Rust BACnetClient.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::BytesMut;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use bacnet_client::client;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryRequest, PriorityFilter};
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::private_transfer::PrivateTransferRequest;
use bacnet_services::text_message::{MessageClass, TextMessageRequest};
use bacnet_services::virtual_terminal::{VTCloseRequest, VTDataRequest, VTOpenRequest};
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_transport::any::AnyTransport;
use bacnet_types::enums::{AcknowledgmentFilter, ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::primitives::BACnetTimeStamp;

use crate::app_transport::AppTransport;
use crate::errors::to_py_err;
use crate::types::{
    PeerAddresses, PyAcknowledgmentFilter, PyBACnetTimeStamp, PyCovNotificationIterator,
    PyDeviceWrite, PyDiscoveredDevice, PyEnableDisable, PyEnrollmentSummaryEventStateFilter,
    PyEventState, PyEventType, PyLifeSafetyOperation, PyMessagePriority, PyObjectIdentifier,
    PyObjectType, PyPropertyIdentifier, PyPropertyValue, PyPropertyWrite, PyReadAccessSpec,
    PyReinitializedState, PyWriteAccessSpec, audit_log_query_ack_to_py,
    audit_log_query_request_from_py, audit_notification_request_from_py, decode_read_ack,
    parse_bip6_address, py_to_rpm_specs, py_to_wpm_specs, rpm_ack_to_py,
};
use state::started;

pub(crate) type CoreClient = client::BACnetClient<AppTransport>;
pub(crate) type ClientInner = Arc<state::ClientState>;

fn validate_write_priority(priority: Option<u8>) -> PyResult<()> {
    bacnet_services::write_property::validate_priority(priority)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// The `min_request_interval_ms` keyword, as `BACnetClient` and the endpoint
/// owners take it (#1535, #1542): more than an hour raises `ValueError` at
/// construction, before any I/O.
pub(crate) fn parse_min_request_interval_ms(interval_ms: u64) -> PyResult<u64> {
    if interval_ms > client::MAX_MIN_REQUEST_INTERVAL_MS {
        return Err(PyValueError::new_err(format!(
            "min_request_interval_ms must be 0..={}, got {interval_ms}",
            client::MAX_MIN_REQUEST_INTERVAL_MS
        )));
    }
    Ok(interval_ms)
}

/// The Who-Is or Who-Has range the `low_limit` and `high_limit` keywords
/// give: both or neither, each from 0 to 4194303 (Clauses 16.9 and 16.10).
/// One alone, a low limit above the high one, or a limit past 4194303 raises
/// `ValueError` before anything is sent, where one limit used to go out as a
/// request for every device (#1483).
fn device_range(
    low_limit: Option<u32>,
    high_limit: Option<u32>,
) -> PyResult<Option<DeviceInstanceRange>> {
    DeviceInstanceRange::from_limits(low_limit, high_limit).map_err(|error| match error {
        bacnet_types::error::Error::OutOfRange(message) => PyValueError::new_err(message),
        bacnet_types::error::Error::Range(refused) => {
            PyValueError::new_err(refused.message().to_string())
        }
        other => PyValueError::new_err(other.to_string()),
    })
}

/// Where the `scope` keyword of `who_is()` and `discover()` sends the
/// Who-Is (#2560), and that of `who_has_by_id()`, `who_has_by_name()` and
/// `who_has_replies()` the Who-Has (#2596), `service` naming which:
/// `"local"`, the default, is this network alone, which no router passes
/// on; `"global"` is every network; a number from 1 to 65534 is that
/// remote network; and any other text is one station's address in the
/// target grammar (`"192.168.1.10"`, `"mstp:5"`), asked by unicast
/// (#2561). Who-Is also accepts a `RoutedTarget` (#2599). Anything else raises before
/// anything is sent: `ValueError` for another number, or for text that
/// names a device instance or a routed target and not a station,
/// `TypeError` for another type. A client on a router's application
/// (`transport`, `"router"`) has no network of its own, so a local or
/// directed request raises `ValueError` there, saying which scopes it can
/// use.
fn discovery_scope(
    scope: Option<&Bound<'_, PyAny>>,
    transport: &str,
    service: &str,
) -> PyResult<ScopeArg> {
    use bacnet_ops::Scope;
    use bacnet_ops::target::{Target, parse_target};
    use pyo3::exceptions::PyTypeError;
    use pyo3::types::{PyBool, PyInt, PyString};
    const EXPECTED: &str =
        "scope is 'local', 'global', a network number from 1 to 65534 or a station's address";
    let local = |what: &str| match transport {
        "router" => Err(PyValueError::new_err(format!(
            "a client on transport='router' has no network of its own for a {what} {service}: \
             pass scope=<network number> for one of the router's networks, or \
             scope='global' for every network"
        ))),
        _ => Ok(()),
    };
    let Some(scope) = scope.filter(|scope| !scope.is_none()) else {
        return local("local").map(|()| ScopeArg::Fixed(Scope::Local));
    };
    if service == "Who-Is"
        && let Ok(target) = scope.cast::<crate::types::PyRoutedTarget>()
    {
        local("directed")?;
        let (network, mac, router) = target.get().parts();
        let station = match router {
            Some(router) => client::Station::routed(router, network, mac),
            None => client::Station::remote(network, mac),
        };
        return Ok(ScopeArg::Fixed(Scope::Directed(station)));
    }
    if let Ok(name) = scope.cast::<PyString>() {
        let text = name.to_str()?.trim();
        return match text {
            "local" => local("local").map(|()| ScopeArg::Fixed(Scope::Local)),
            "global" => Ok(ScopeArg::Fixed(Scope::Global)),
            // A station by its address. A bare number in text is a device
            // instance to the grammar, and a scope is a place, not a
            // device: both it and a routed target are refused.
            station => match parse_target(station) {
                Ok(Target::Mac(_) | Target::Bip6(_)) => {
                    local("directed").map(|()| ScopeArg::Directed(station.to_owned()))
                }
                Ok(_) => Err(PyValueError::new_err(format!(
                    "{EXPECTED}, and '{station}' names a device or a routed target: for a \
                     remote network pass its number, scope=<network>"
                ))),
                Err(_) => Err(PyValueError::new_err(format!(
                    "{EXPECTED}, not '{station}'"
                ))),
            },
        };
    }
    if scope.is_instance_of::<PyInt>() && !scope.is_instance_of::<PyBool>() {
        return match scope.extract::<u16>() {
            Ok(network @ 1..=65534) => Ok(ScopeArg::Fixed(Scope::Network(network))),
            _ => Err(PyValueError::new_err(format!("{EXPECTED}, not {scope}"))),
        };
    }
    Err(PyTypeError::new_err(format!(
        "{EXPECTED}, not {}",
        scope.get_type().name()?
    )))
}

/// A `scope` keyword as checked when the call was made: a scope as it is,
/// or a station's address still to be read for the started client's link.
enum ScopeArg {
    /// Local, global or one network.
    Fixed(bacnet_ops::Scope),
    /// One station, by its address in the target grammar.
    Directed(String),
}

impl ScopeArg {
    /// The scope on `client`: a station's address read for its link (a
    /// B/IPv6 socket address asks the node its VMAC). Text the link can't
    /// reach raises `ValueError`, and nothing is sent.
    async fn on(self, client: &CoreClient) -> PyResult<bacnet_ops::Scope> {
        use bacnet_ops::{DeviceAddress, Scope};
        let station = match self {
            Self::Fixed(scope) => return Ok(scope),
            Self::Directed(station) => station,
        };
        let refusal = "a directed scope names a station's address";
        let mac = bacnet_ops::target::address_mac(client.transport(), &station, refusal)
            .await
            .map_err(|reason| PyValueError::new_err(crate::high_level::link_hint(reason)))?;
        DeviceAddress::local(&mac)
            .map(|address| Scope::Directed(address.into()))
            .map_err(PyValueError::new_err)
    }
}

/// The `wait` keyword of `discover()`, `who_has_replies()` and
/// `discover_routers()`: how long the call listens for answers, in
/// seconds from 0 to 3600 (an hour). Anything else, NaN included, raises
/// `ValueError` when the call is made.
fn discover_wait(seconds: f64) -> PyResult<std::time::Duration> {
    if !(0.0..=3600.0).contains(&seconds) {
        return Err(PyValueError::new_err(format!(
            "wait is a number of seconds from 0 to 3600, not {seconds}"
        )));
    }
    Ok(std::time::Duration::from_secs_f64(seconds))
}

/// A scoped send's error as Python sees it: the link's as the client
/// raises it, a refused scope as `ValueError`.
fn scoped_error(error: bacnet_ops::OpsError) -> PyErr {
    match error {
        bacnet_ops::OpsError::Client(error) => to_py_err(error),
        other => PyValueError::new_err(other.to_string()),
    }
}

/// Send one Who-Is to `scope` on `client`, for `range`: the scope was
/// checked when the call was made, so a failure here is the link's.
async fn send_who_is(
    client: &CoreClient,
    scope: &bacnet_ops::Scope,
    range: Option<DeviceInstanceRange>,
) -> PyResult<()> {
    bacnet_ops::raw::who_is(client, scope, range)
        .await
        .map_err(scoped_error)
}

/// Send one Who-Has for `object` to `scope` on `client`, for `range`, as
/// [`send_who_is`] sends a Who-Is.
async fn send_who_has(
    client: &CoreClient,
    scope: &bacnet_ops::Scope,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
) -> PyResult<()> {
    bacnet_ops::raw::who_has(client, scope, object, range)
        .await
        .map_err(scoped_error)
}

/// Async BACnet client for reading/writing properties on remote devices.
///
/// Usage:
/// ```python
/// async with BACnetClient("0.0.0.0", 47808) as client:
///     value = await client.read_property("192.168.1.100:47808", oid, pid)
///     print(value.tag, value.value)
/// ```
///
/// Supports multiple transports via the `transport` parameter:
/// - `"bip"` (default): BACnet/IP over UDP
/// - `"ipv6"`: BACnet/IPv6 over UDP multicast. Peers are named by VMAC
///   (`"bip6:1234"`); `resolve_vmac` finds the VMAC at an `"[ip]:port"`.
///   `foreign_device=` with a B/IPv6 BBMD, and `ipv6_device_instance`,
///   registers it as a foreign device (#1941).
/// - `"sc"`: BACnet/SC over TLS WebSocket (requires `sc_hub`, `sc_vmac`,
///   `sc_ca_cert`, `sc_client_cert`, `sc_client_key`, and persistent `sc_device_uuid`)
/// - `"mstp"`: BACnet MS/TP over RS-485 (requires `serial_port`)
/// - `"router"`: the application entity of `router=`, a `BACnetRouter`
///   built with `application=` (#2071). Address devices by network, through
///   discovery and the `_from_device`/`_to_device` methods: an `ip:port`
///   names no network, so a send to one fails.
///
/// SC credential paths must be nonempty at construction (ValueError otherwise).
/// Files are loaded on async entry; invalid TLS configuration raises RuntimeError
/// before dialing. No system trust or unauthenticated-client fallback is used.
#[pyclass(name = "BACnetClient", module = "rusty_bacnet")]
pub struct BACnetClient {
    inner: ClientInner,
    /// A client on an endpoint's link (#2617): its endpoint starts it,
    /// and `async with` only enters it.
    seated: bool,
    transport_type: String,
    // BIP config
    interface: String,
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
    apdu_timeout_ms: u64,
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
    reconnect: Option<bacnet_transport::sc::ScReconnectConfig>,
    /// The failover hub's URL (#1829); requires `reconnect`.
    sc_failover_hub: Option<String>,
    /// The bound on each hub dial and Connect-Request (#1829).
    sc_connect_timeout_ms: Option<u64>,
    // IPv6 config
    ipv6_interface: Option<String>,
    /// The device instance whose VMAC an ipv6 client uses (H.7.2); `None`
    /// picks a random VMAC at each start.
    ipv6_device_instance: Option<u32>,
    /// The MS/TP keywords, `serial_port` included.
    mstp: crate::mstp_py::MstpOptions,
    /// The router whose application entity `transport="router"` runs on
    /// (#2071).
    router: Option<crate::router::application::RouterApplication>,
    /// Least time between confirmed requests to one destination (#1535).
    min_request_interval_ms: u64,
    /// The priorities `relinquish` may use (#2561): 1, 2, 5 and 6 refused
    /// unless `write_policy=` allows them.
    write_policy: bacnet_ops::WritePolicy,
    /// Where a `device()` handle made from a bare instance asks for it
    /// (#2561): `None` is this network alone, and on a router's
    /// application, which has no network of its own, nowhere.
    resolve_scope: Option<bacnet_ops::ResolveScope>,
}

mod client_methods {
    mod bbmd_management;
    mod cov_discovery;
    mod discovery_streams;
    mod enrollment_alarm_covmulti_who_writegroup;
    mod export_223;
    mod file_list_private_text_life;
    mod foreign_registration;
    mod frames;
    mod i_am_arrivals;
    mod lifecycle;
    mod log_read;
    mod object_device_alarm;
    mod priorities;
    mod read_write;
    mod routed_paths;
    mod routing_table;
    mod sc_peer;
    mod vt_audit_time_directed;
}
mod handles;
mod routed;
mod seated;
mod state;

pub(crate) use handles::DeviceTie;
pub(crate) use state::{Held, Liveness, close};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an optional `MessageClass` from Python arguments.
fn build_message_class(
    mc_type: Option<String>,
    mc_value: Option<Bound<'_, PyAny>>,
) -> PyResult<Option<MessageClass>> {
    match mc_type.as_deref() {
        Some("numeric") => {
            let v = mc_value
                .ok_or_else(|| {
                    PyValueError::new_err("message_class_value required for numeric class")
                })?
                .extract::<u32>()?;
            Ok(Some(MessageClass::Numeric(v)))
        }
        Some("text") => {
            let v = mc_value
                .ok_or_else(|| {
                    PyValueError::new_err("message_class_value required for text class")
                })?
                .extract::<String>()?;
            Ok(Some(MessageClass::Text(v)))
        }
        Some(other) => Err(PyValueError::new_err(format!(
            "message_class_type must be 'numeric', 'text', or None, got '{other}'"
        ))),
        None => Ok(None),
    }
}
