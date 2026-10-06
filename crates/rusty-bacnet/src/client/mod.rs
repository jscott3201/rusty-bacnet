//! Python BACnetClient — async wrapper around the Rust BACnetClient.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::BytesMut;
use pyo3::exceptions::{PyDeprecationWarning, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use tokio::sync::Mutex;

use bacnet_client::client;
use bacnet_client::client::WriteGroupDestination;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
type ClientInner =
    Arc<Mutex<Option<Arc<client::BACnetClient<AnyTransport<crate::mstp_py::PySerial>>>>>>;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::enrollment_summary::{
    GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest, PriorityFilter,
};
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::private_transfer::{PrivateTransferAck, PrivateTransferRequest};
use bacnet_services::text_message::{MessageClass, TextMessageRequest};
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::enums::{AcknowledgmentFilter, ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::primitives::BACnetTimeStamp;

use crate::errors::to_py_err;
use crate::types::{
    audit_log_query_ack_to_py, audit_log_query_request_from_py, audit_notification_request_from_py,
    decode_read_ack, parse_address, py_to_rpm_specs, py_to_wpm_specs, rpm_ack_to_py,
    PyAcknowledgmentFilter, PyBACnetTimeStamp, PyCovNotificationIterator, PyDeviceWrite,
    PyDiscoveredDevice, PyEnableDisable, PyEnrollmentSummaryEventStateFilter, PyEventState,
    PyEventType, PyLifeSafetyOperation, PyMessagePriority, PyObjectIdentifier, PyObjectType,
    PyPropertyIdentifier, PyPropertyValue, PyPropertyWrite, PyReadAccessSpec, PyReinitializedState,
    PyWriteAccessSpec,
};

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
        other => PyValueError::new_err(other.to_string()),
    })
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
/// - `"ipv6"`: BACnet/IPv6 over UDP multicast
/// - `"sc"`: BACnet/SC over TLS WebSocket (requires `sc_hub`, `sc_vmac`,
///   `sc_ca_cert`, `sc_client_cert`, `sc_client_key`, and persistent `sc_device_uuid`)
/// - `"mstp"`: BACnet MS/TP over RS-485 (requires `serial_port`)
///
/// SC credential paths must be nonempty at construction (ValueError otherwise).
/// Files are loaded on async entry; invalid TLS configuration raises RuntimeError
/// before dialing. No system trust or unauthenticated-client fallback is used.
#[pyclass(name = "BACnetClient", module = "rusty_bacnet")]
pub struct BACnetClient {
    inner: ClientInner,
    transport_type: String,
    // BIP config
    interface: String,
    port: u16,
    broadcast_address: String,
    /// See `BipTransport::set_share_port_by_address` (#1538).
    share_port_by_address: bool,
    apdu_timeout_ms: u64,
    // SC config
    sc_hub: Option<String>,
    sc_vmac: Option<Vec<u8>>,
    sc_device_uuid: [u8; 16],
    sc_ca_cert: Option<String>,
    sc_client_cert: Option<String>,
    sc_client_key: Option<String>,
    sc_heartbeat_interval_ms: Option<u64>,
    sc_heartbeat_timeout_ms: Option<u64>,
    // IPv6 config
    ipv6_interface: Option<String>,
    // MS/TP config
    serial_port: Option<String>,
    mstp_baud: u32,
    mstp_mac: u8,
    mstp_max_master: u8,
    mstp_max_info_frames: u8,
    /// Least time between confirmed requests to one destination (#1535).
    min_request_interval_ms: u64,
}

mod client_methods {
    mod cov_discovery;
    mod enrollment_alarm_covmulti_who_writegroup;
    mod file_list_private_text_life;
    mod lifecycle;
    mod log_read;
    mod object_device_alarm;
    mod read_write;
    mod vt_audit_time_directed;
}

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
