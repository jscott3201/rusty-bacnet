//! Python BACnetClient — async wrapper around the Rust BACnetClient.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::BytesMut;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use tokio::sync::Mutex;

use bacnet_client::client;
use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::audit::AuditLogQueryRequest;

type ClientInner = Arc<Mutex<Option<Arc<client::BACnetClient<AnyTransport<NoSerial>>>>>>;
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
use bacnet_services::read_range::RangeSpec;
use bacnet_services::text_message::{MessageClass, TextMessageRequest};
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::mstp::NoSerial;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};

use crate::errors::to_py_err;
use crate::types::{
    parse_address, py_to_rpm_specs, py_to_wpm_specs, rpm_ack_to_py, PyCovNotificationIterator,
    PyDiscoveredDevice, PyEnableDisable, PyEventState, PyEventType, PyLifeSafetyOperation,
    PyMessagePriority, PyObjectIdentifier, PyObjectType, PyPropertyIdentifier, PyPropertyValue,
    PyReinitializedState,
};

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
/// - `"sc"`: BACnet/SC over TLS WebSocket (requires `sc_hub`, `sc_vmac`)
#[pyclass(name = "BACnetClient")]
pub struct BACnetClient {
    inner: ClientInner,
    transport_type: String,
    // BIP config
    interface: String,
    port: u16,
    broadcast_address: String,
    apdu_timeout_ms: u64,
    // SC config
    sc_hub: Option<String>,
    sc_vmac: Option<Vec<u8>>,
    sc_ca_cert: Option<String>,
    sc_client_cert: Option<String>,
    sc_client_key: Option<String>,
    sc_heartbeat_interval_ms: Option<u64>,
    sc_heartbeat_timeout_ms: Option<u64>,
    // IPv6 config
    ipv6_interface: Option<String>,
}

mod client_methods {
    mod cov_discovery;
    mod enrollment_alarm_covmulti_who_writegroup;
    mod file_list_private_text_life;
    mod lifecycle;
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
