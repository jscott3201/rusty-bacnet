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

#[pymethods]
impl BACnetClient {
    #[new]
    #[pyo3(signature = (
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        apdu_timeout_ms=6000,
        transport="bip",
        sc_hub=None,
        sc_vmac=None,
        sc_ca_cert=None,
        sc_client_cert=None,
        sc_client_key=None,
        sc_heartbeat_interval_ms=None,
        sc_heartbeat_timeout_ms=None,
        ipv6_interface=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        interface: &str,
        port: u16,
        broadcast_address: &str,
        apdu_timeout_ms: u64,
        transport: &str,
        sc_hub: Option<String>,
        sc_vmac: Option<Vec<u8>>,
        sc_ca_cert: Option<String>,
        sc_client_cert: Option<String>,
        sc_client_key: Option<String>,
        sc_heartbeat_interval_ms: Option<u64>,
        sc_heartbeat_timeout_ms: Option<u64>,
        ipv6_interface: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            apdu_timeout_ms,
            sc_hub,
            sc_vmac,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            ipv6_interface,
        }
    }

    /// Start the client (called by `async with`).
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let inner = slf.borrow().inner.clone();
        let transport_type = slf.borrow().transport_type.clone();
        let interface_str = slf.borrow().interface.clone();
        let port = slf.borrow().port;
        let broadcast_str = slf.borrow().broadcast_address.clone();
        let timeout_ms = slf.borrow().apdu_timeout_ms;
        let sc_hub = slf.borrow().sc_hub.clone();
        let sc_vmac = slf.borrow().sc_vmac.clone();
        let sc_ca_cert = slf.borrow().sc_ca_cert.clone();
        let sc_client_cert = slf.borrow().sc_client_cert.clone();
        let sc_client_key = slf.borrow().sc_client_key.clone();
        let sc_heartbeat_interval_ms = slf.borrow().sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = slf.borrow().sc_heartbeat_timeout_ms;
        let ipv6_interface = slf.borrow().ipv6_interface.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let transport: AnyTransport<NoSerial> = match transport_type.as_str() {
                "bip" => {
                    let interface: Ipv4Addr = interface_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid interface: {e}")))?;
                    let broadcast: Ipv4Addr = broadcast_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid broadcast: {e}")))?;
                    AnyTransport::Bip(BipTransport::new(interface, port, broadcast))
                }
                "ipv6" => {
                    let iface_str = ipv6_interface.as_deref().unwrap_or("::");
                    let interface: std::net::Ipv6Addr = iface_str.parse().map_err(|e| {
                        PyRuntimeError::new_err(format!("invalid IPv6 interface: {e}"))
                    })?;
                    AnyTransport::Bip6(Bip6Transport::new(interface, port, None))
                }
                "sc" => {
                    let hub_url = sc_hub.ok_or_else(|| {
                        PyRuntimeError::new_err("sc_hub is required for SC transport")
                    })?;
                    let vmac_bytes = sc_vmac.ok_or_else(|| {
                        PyRuntimeError::new_err("sc_vmac is required for SC transport")
                    })?;
                    if vmac_bytes.len() != 6 {
                        return Err(PyRuntimeError::new_err("sc_vmac must be exactly 6 bytes"));
                    }
                    let mut vmac = [0u8; 6];
                    vmac.copy_from_slice(&vmac_bytes);

                    let tls_config = crate::tls::build_client_tls_config(
                        sc_ca_cert.as_deref(),
                        sc_client_cert.as_deref(),
                        sc_client_key.as_deref(),
                    )
                    .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?;

                    let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config)
                        .await
                        .map_err(to_py_err)?;

                    let mut sc = bacnet_transport::sc::ScTransport::new(ws, vmac);
                    if let Some(ms) = sc_heartbeat_interval_ms {
                        sc = sc.with_heartbeat_interval_ms(ms);
                    }
                    if let Some(ms) = sc_heartbeat_timeout_ms {
                        sc = sc.with_heartbeat_timeout_ms(ms);
                    }
                    AnyTransport::Sc(Box::new(sc))
                }
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unknown transport: '{other}'. Use 'bip', 'ipv6', or 'sc'"
                    )));
                }
            };

            let c = client::BACnetClient::generic_builder()
                .transport(transport)
                .apdu_timeout_ms(timeout_ms)
                .build()
                .await
                .map_err(to_py_err)?;

            *inner.lock().await = Some(Arc::new(c));
            Ok(self_ref)
        })
    }

    /// Stop the client (called by `async with` exit).
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Bound<'py, PyAny>>,
        _exc_val: Option<Bound<'py, PyAny>>,
        _exc_tb: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let arc = {
                let mut guard = inner.lock().await;
                guard.take()
            };
            if let Some(arc) = arc {
                // Try to get exclusive ownership for clean stop
                match Arc::try_unwrap(arc) {
                    Ok(mut c) => {
                        if let Err(e) = c.stop().await {
                            eprintln!("BACnetClient stop error in __aexit__: {e}");
                        }
                    }
                    Err(_arc) => {
                        // Other async operations still hold references;
                        // cleanup will happen when they complete and drop the Arc.
                    }
                }
            }
            Ok(())
        })
    }

    /// Read a property from a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port" (e.g. "192.168.1.100:47808")
    ///     object_id: Target object identifier
    ///     property_id: Property to read
    ///     array_index: Optional array index (for array properties)
    ///
    /// Returns: PropertyValue with `.tag`, `.value` accessors
    #[pyo3(signature = (address, object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed

            let ack = c
                .read_property(&mac, oid, pid, array_index)
                .await
                .map_err(to_py_err)?;

            // Decode application-tagged value bytes → PropertyValue
            let (value, _) = decode_application_value(&ack.property_value, 0).map_err(to_py_err)?;

            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Write a property on a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port"
    ///     object_id: Target object identifier
    ///     property_id: Property to write
    ///     value: PropertyValue to write (e.g. `PropertyValue.real(72.5)`)
    ///     priority: Optional priority (1-16, for commandable properties)
    ///     array_index: Optional array index
    #[pyo3(signature = (address, object_id, property_id, value, priority=None, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Some(p) = priority {
            if !(1..=16).contains(&p) {
                return Err(PyValueError::new_err(format!(
                    "priority must be 1-16, got {p}"
                )));
            }
        }

        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed

            // Encode PropertyValue to application-tagged bytes
            let mut value_buf = BytesMut::new();
            encode_property_value(&mut value_buf, &prop_value).map_err(to_py_err)?;

            c.write_property(&mac, oid, pid, array_index, value_buf.to_vec(), priority)
                .await
                .map_err(to_py_err)?;

            Ok(())
        })
    }

    /// Send a WhoIs broadcast to discover devices.
    #[pyo3(signature = (low_limit=None, high_limit=None))]
    fn who_is<'py>(
        &self,
        py: Python<'py>,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed
            c.who_is(low_limit, high_limit).await.map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // RPM / WPM
    // -----------------------------------------------------------------------

    /// Read multiple properties from multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    #[allow(clippy::type_complexity)]
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_rpm_specs(specs);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_property_multiple(&mac, rust_specs)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write multiple properties to multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    #[allow(clippy::type_complexity)]
    fn write_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<(
            PyObjectIdentifier,
            Vec<(
                PyPropertyIdentifier,
                PyPropertyValue,
                Option<u8>,
                Option<u32>,
            )>,
        )>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_wpm_specs(specs);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.write_property_multiple(&mac, rust_specs)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // COV
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None))]
    #[allow(clippy::too_many_arguments)]
    fn subscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.subscribe_cov(
                &mac,
                subscriber_process_identifier,
                oid,
                confirmed,
                lifetime,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Unsubscribe from COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier))]
    fn unsubscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.unsubscribe_cov(&mac, subscriber_process_identifier, oid)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get an async iterator yielding incoming COV notifications.
    fn cov_notifications<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let c = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("client not started — use 'async with'"))?;
            let rx = c.cov_notifications();
            Ok(PyCovNotificationIterator::new(rx))
        })
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Send a WhoHas broadcast to find an object by identifier.
    #[pyo3(signature = (object_id, low_limit=None, high_limit=None))]
    fn who_has_by_id<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_has(WhoHasObject::Identifier(oid), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send a WhoHas broadcast to find an object by name.
    #[pyo3(signature = (name, low_limit=None, high_limit=None))]
    fn who_has_by_name<'py>(
        &self,
        py: Python<'py>,
        name: String,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_has(WhoHasObject::Name(name), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get a list of all discovered devices.
    fn discovered_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let devices = c.discovered_devices().await;
            Ok(devices
                .into_iter()
                .map(PyDiscoveredDevice::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Look up a discovered device by instance number.
    fn get_device<'py>(&self, py: Python<'py>, instance: u32) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            Ok(c.get_device(instance)
                .await
                .map(PyDiscoveredDevice::from_rust))
        })
    }

    /// Clear the discovered devices table.
    fn clear_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.clear_devices().await;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Object management
    // -----------------------------------------------------------------------

    /// Delete an object on a remote device.
    #[pyo3(signature = (address, object_id))]
    fn delete_object<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.delete_object(&mac, oid).await.map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Create an object on a remote device.
    ///
    /// `object_specifier` accepts either an `ObjectType` (server picks instance)
    /// or an `ObjectIdentifier` (specific instance).
    /// `initial_values` is an optional list of `(PropertyIdentifier, PropertyValue, priority, array_index)` tuples.
    #[pyo3(signature = (address, object_specifier, initial_values=None))]
    #[allow(clippy::type_complexity)]
    fn create_object<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_specifier: Bound<'py, PyAny>,
        initial_values: Option<
            Vec<(
                PyPropertyIdentifier,
                PyPropertyValue,
                Option<u8>,
                Option<u32>,
            )>,
        >,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        // Determine specifier type
        let specifier = if let Ok(ot) = object_specifier.extract::<PyObjectType>() {
            ObjectSpecifier::Type(ot.to_rust())
        } else if let Ok(oid) = object_specifier.extract::<PyObjectIdentifier>() {
            ObjectSpecifier::Identifier(oid.to_rust())
        } else {
            return Err(PyValueError::new_err(
                "object_specifier must be ObjectType or ObjectIdentifier",
            ));
        };

        let init_vals: Vec<BACnetPropertyValue> = initial_values
            .unwrap_or_default()
            .into_iter()
            .map(|(pid, val, priority, array_index)| {
                let mut buf = BytesMut::new();
                let _ = encode_property_value(&mut buf, &val.inner);
                BACnetPropertyValue {
                    property_identifier: pid.to_rust(),
                    property_array_index: array_index,
                    value: buf.to_vec(),
                    priority,
                }
            })
            .collect();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .create_object(&mac, specifier, init_vals)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // Device management
    // -----------------------------------------------------------------------

    /// Send a DeviceCommunicationControl request.
    #[pyo3(signature = (address, enable_disable, time_duration=None, password=None))]
    fn device_communication_control<'py>(
        &self,
        py: Python<'py>,
        address: String,
        enable_disable: PyEnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let ed = enable_disable.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.device_communication_control(&mac, ed, time_duration, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send a ReinitializeDevice request.
    #[pyo3(signature = (address, reinitialized_state, password=None))]
    fn reinitialize_device<'py>(
        &self,
        py: Python<'py>,
        address: String,
        reinitialized_state: PyReinitializedState,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let state = reinitialized_state.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.reinitialize_device(&mac, state, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Alarms / Events
    // -----------------------------------------------------------------------

    /// Acknowledge an alarm on a remote device.
    #[pyo3(signature = (address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, acknowledgment_source))]
    #[allow(clippy::too_many_arguments)]
    fn acknowledge_alarm<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledging_process_identifier: u32,
        event_object_identifier: PyObjectIdentifier,
        event_state_acknowledged: u32,
        acknowledgment_source: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = event_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.acknowledge_alarm(
                &mac,
                acknowledging_process_identifier,
                oid,
                event_state_acknowledged,
                &acknowledgment_source,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get event information from a remote device.
    #[pyo3(signature = (address, last_received_object_identifier=None))]
    fn get_event_information<'py>(
        &self,
        py: Python<'py>,
        address: String,
        last_received_object_identifier: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let last_oid = last_received_object_identifier.map(|o| o.to_rust());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .get_event_information(&mac, last_oid)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // ReadRange
    // -----------------------------------------------------------------------

    /// Read a range of items from a list or log object.
    ///
    /// `range_type` is `"position"`, `"sequence"`, or `None` (no range).
    #[pyo3(signature = (address, object_id, property_id, array_index=None, range_type=None, reference_index=None, reference_seq=None, count=None))]
    #[allow(clippy::too_many_arguments)]
    fn read_range<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
        range_type: Option<String>,
        reference_index: Option<u32>,
        reference_seq: Option<u32>,
        count: Option<i32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        let range = match range_type.as_deref() {
            Some("position") => Some(RangeSpec::ByPosition {
                reference_index: reference_index.unwrap_or(0),
                count: count.unwrap_or(0),
            }),
            Some("sequence") => Some(RangeSpec::BySequenceNumber {
                reference_seq: reference_seq.unwrap_or(0),
                count: count.unwrap_or(0),
            }),
            Some(other) => {
                return Err(PyValueError::new_err(format!(
                    "range_type must be 'position', 'sequence', or None, got '{other}'"
                )));
            }
            None => None,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_range(&mac, oid, pid, array_index, range)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item(
                    "object_id",
                    PyObjectIdentifier::from_rust(ack.object_identifier),
                )?;
                dict.set_item(
                    "property_id",
                    PyPropertyIdentifier {
                        inner: ack.property_identifier,
                    },
                )?;
                dict.set_item("array_index", ack.property_array_index)?;
                dict.set_item("result_flags", ack.result_flags)?;
                dict.set_item("item_count", ack.item_count)?;
                dict.set_item("item_data", PyBytes::new(py, &ack.item_data))?;
                Ok(dict.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // File services
    // -----------------------------------------------------------------------

    /// Read from a file object (stream or record access).
    ///
    /// `access_method` is `"stream"` or `"record"`.
    #[pyo3(signature = (address, file_identifier, access_method, start_position=0, requested_octet_count=0, start_record=0, requested_record_count=0))]
    #[allow(clippy::too_many_arguments)]
    fn atomic_read_file<'py>(
        &self,
        py: Python<'py>,
        address: String,
        file_identifier: PyObjectIdentifier,
        access_method: String,
        start_position: i32,
        requested_octet_count: u32,
        start_record: i32,
        requested_record_count: u32,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let fid = file_identifier.to_rust();

        let access = match access_method.as_str() {
            "stream" => FileAccessMethod::Stream {
                file_start_position: start_position,
                requested_octet_count,
            },
            "record" => FileAccessMethod::Record {
                file_start_record: start_record,
                requested_record_count,
            },
            other => {
                return Err(PyValueError::new_err(format!(
                    "access_method must be 'stream' or 'record', got '{other}'"
                )));
            }
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .atomic_read_file(&mac, fid, access)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    /// Write to a file object (stream or record access).
    ///
    /// `access_method` is `"stream"` or `"record"`.
    #[pyo3(signature = (address, file_identifier, access_method, start_position=0, file_data=vec![], start_record=0, record_count=0, file_record_data=None))]
    #[allow(clippy::too_many_arguments)]
    fn atomic_write_file<'py>(
        &self,
        py: Python<'py>,
        address: String,
        file_identifier: PyObjectIdentifier,
        access_method: String,
        start_position: i32,
        file_data: Vec<u8>,
        start_record: i32,
        record_count: u32,
        file_record_data: Option<Vec<Vec<u8>>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let fid = file_identifier.to_rust();

        let access = match access_method.as_str() {
            "stream" => FileWriteAccessMethod::Stream {
                file_start_position: start_position,
                file_data,
            },
            "record" => FileWriteAccessMethod::Record {
                file_start_record: start_record,
                record_count,
                file_record_data: file_record_data.unwrap_or_default(),
            },
            other => {
                return Err(PyValueError::new_err(format!(
                    "access_method must be 'stream' or 'record', got '{other}'"
                )));
            }
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .atomic_write_file(&mac, fid, access)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // List manipulation
    // -----------------------------------------------------------------------

    /// Add elements to a list property.
    #[pyo3(signature = (address, object_id, property_id, list_of_elements, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn add_list_element<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        list_of_elements: Vec<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.add_list_element(&mac, oid, pid, array_index, list_of_elements)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Remove elements from a list property.
    #[pyo3(signature = (address, object_id, property_id, list_of_elements, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn remove_list_element<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        list_of_elements: Vec<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.remove_list_element(&mac, oid, pid, array_index, list_of_elements)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // PrivateTransfer
    // -----------------------------------------------------------------------

    /// Send a ConfirmedPrivateTransfer request.
    ///
    /// Returns a dict with `vendor_id`, `service_number`, and optional `result_block` (bytes).
    #[pyo3(signature = (address, vendor_id, service_number, service_parameters=None))]
    fn confirmed_private_transfer<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vendor_id: u32,
        service_number: u32,
        service_parameters: Option<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = PrivateTransferRequest {
                vendor_id,
                service_number,
                service_parameters,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(
                    &mac,
                    ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER,
                    &buf,
                )
                .await
                .map_err(to_py_err)?;
            let ack = PrivateTransferAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("vendor_id", ack.vendor_id)?;
                dict.set_item("service_number", ack.service_number)?;
                match ack.result_block {
                    Some(ref data) => {
                        dict.set_item("result_block", PyBytes::new(py, data))?;
                    }
                    None => {
                        dict.set_item("result_block", py.None())?;
                    }
                }
                Ok(dict.into_any().unbind())
            })
        })
    }

    /// Send an UnconfirmedPrivateTransfer request.
    #[pyo3(signature = (address, vendor_id, service_number, service_parameters=None))]
    fn unconfirmed_private_transfer<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vendor_id: u32,
        service_number: u32,
        service_parameters: Option<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = PrivateTransferRequest {
                vendor_id,
                service_number,
                service_parameters,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.unconfirmed_request(
                &mac,
                UnconfirmedServiceChoice::UNCONFIRMED_PRIVATE_TRANSFER,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // TextMessage
    // -----------------------------------------------------------------------

    /// Send a ConfirmedTextMessage request.
    ///
    /// `message_class_type` is `"numeric"` or `"text"` (or None for no class).
    /// `message_class_value` is the numeric value or text string.
    #[pyo3(signature = (address, source_device, message_priority, message, message_class_type=None, message_class_value=None))]
    #[allow(clippy::too_many_arguments)]
    fn confirmed_text_message<'py>(
        &self,
        py: Python<'py>,
        address: String,
        source_device: PyObjectIdentifier,
        message_priority: PyMessagePriority,
        message: String,
        message_class_type: Option<String>,
        message_class_value: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let src = source_device.to_rust();
        let priority = message_priority.to_rust();
        let mc = build_message_class(message_class_type, message_class_value)?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = TextMessageRequest {
                source_device: src,
                message_class: mc,
                message_priority: priority,
                message,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.confirmed_request(&mac, ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send an UnconfirmedTextMessage request.
    #[pyo3(signature = (address, source_device, message_priority, message, message_class_type=None, message_class_value=None))]
    #[allow(clippy::too_many_arguments)]
    fn unconfirmed_text_message<'py>(
        &self,
        py: Python<'py>,
        address: String,
        source_device: PyObjectIdentifier,
        message_priority: PyMessagePriority,
        message: String,
        message_class_type: Option<String>,
        message_class_value: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let src = source_device.to_rust();
        let priority = message_priority.to_rust();
        let mc = build_message_class(message_class_type, message_class_value)?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = TextMessageRequest {
                source_device: src,
                message_class: mc,
                message_priority: priority,
                message,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.unconfirmed_request(
                &mac,
                UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // LifeSafetyOperation
    // -----------------------------------------------------------------------

    /// Send a LifeSafetyOperation request.
    #[pyo3(signature = (address, requesting_process_identifier, requesting_source, operation, object_identifier=None))]
    #[allow(clippy::too_many_arguments)]
    fn life_safety_operation<'py>(
        &self,
        py: Python<'py>,
        address: String,
        requesting_process_identifier: u32,
        requesting_source: String,
        operation: PyLifeSafetyOperation,
        object_identifier: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let op = operation.to_rust();
        let oid = object_identifier.map(|o| o.to_rust());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = LifeSafetyOperationRequest {
                requesting_process_identifier,
                requesting_source,
                request: op,
                object_identifier: oid,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.confirmed_request(&mac, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // GetEnrollmentSummary
    // -----------------------------------------------------------------------

    /// Get enrollment summary from a remote device.
    ///
    /// `acknowledgment_filter`: 0=all, 1=acked, 2=not-acked.
    /// Returns a list of dicts with `object_id`, `event_type`, `event_state`, `priority`, `notification_class`.
    #[pyo3(signature = (address, acknowledgment_filter=0, event_state_filter=None, event_type_filter=None, min_priority=None, max_priority=None, notification_class_filter=None))]
    #[allow(clippy::too_many_arguments)]
    fn get_enrollment_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledgment_filter: u32,
        event_state_filter: Option<PyEventState>,
        event_type_filter: Option<PyEventType>,
        min_priority: Option<u8>,
        max_priority: Option<u8>,
        notification_class_filter: Option<u16>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let es = event_state_filter.map(|e| e.to_rust());
        let et = event_type_filter.map(|e| e.to_rust());
        let pf = match (min_priority, max_priority) {
            (Some(min), Some(max)) => Some(PriorityFilter {
                min_priority: min,
                max_priority: max,
            }),
            _ => None,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = GetEnrollmentSummaryRequest {
                acknowledgment_filter,
                event_state_filter: es,
                event_type_filter: et,
                priority_filter: pf,
                notification_class_filter,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY, &buf)
                .await
                .map_err(to_py_err)?;
            let ack = GetEnrollmentSummaryAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "event_type",
                        PyEventType {
                            inner: entry.event_type,
                        },
                    )?;
                    dict.set_item(
                        "event_state",
                        PyEventState {
                            inner: entry.event_state,
                        },
                    )?;
                    dict.set_item("priority", entry.priority)?;
                    dict.set_item("notification_class", entry.notification_class)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // GetAlarmSummary
    // -----------------------------------------------------------------------

    /// Get alarm summary from a remote device (deprecated service).
    ///
    /// Returns a list of dicts with `object_id`, `alarm_state`, `acknowledged_transitions`.
    #[pyo3(signature = (address,))]
    fn get_alarm_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[])
                .await
                .map_err(to_py_err)?;
            let ack = GetAlarmSummaryAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "alarm_state",
                        PyEventState {
                            inner: entry.alarm_state,
                        },
                    )?;
                    let (unused_bits, ref data) = entry.acknowledged_transitions;
                    let trans_dict = PyDict::new(py);
                    trans_dict.set_item("unused_bits", unused_bits)?;
                    trans_dict.set_item("data", PyBytes::new(py, data))?;
                    dict.set_item("acknowledged_transitions", trans_dict)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // SubscribeCOVPropertyMultiple
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for multiple properties on multiple objects.
    ///
    /// `specs` is a list of `(ObjectIdentifier, [(PropertyIdentifier, array_index, cov_increment, timestamped), ...])`.
    /// `cov_increment` is an optional float; `timestamped` is a bool.
    #[pyo3(signature = (address, subscriber_process_identifier, specs, max_notification_delay=None, issue_confirmed_notifications=None))]
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn subscribe_cov_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        specs: Vec<(
            PyObjectIdentifier,
            Vec<(PyPropertyIdentifier, Option<u32>, Option<f32>, bool)>,
        )>,
        max_notification_delay: Option<u32>,
        issue_confirmed_notifications: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs: Vec<COVSubscriptionSpecification> = specs
            .into_iter()
            .map(|(oid, refs)| COVSubscriptionSpecification {
                monitored_object_identifier: oid.to_rust(),
                list_of_cov_references: refs
                    .into_iter()
                    .map(|(pid, idx, inc, ts)| COVReference {
                        monitored_property: bacnet_services::common::PropertyReference {
                            property_identifier: pid.to_rust(),
                            property_array_index: idx,
                        },
                        cov_increment: inc,
                        timestamped: ts,
                    })
                    .collect(),
            })
            .collect();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = SubscribeCOVPropertyMultipleRequest {
                subscriber_process_identifier,
                max_notification_delay,
                issue_confirmed_notifications,
                list_of_cov_subscription_specifications: rust_specs,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.confirmed_request(
                &mac,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Who-Am-I
    // -----------------------------------------------------------------------

    /// Broadcast a Who-Am-I request.
    fn who_am_i<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = WhoAmIRequest;
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_AM_I, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // WriteGroup
    // -----------------------------------------------------------------------

    /// Send a WriteGroup request (unconfirmed).
    ///
    /// `change_list` is a list of `(channel_oid_or_none, override_priority_or_none, value_bytes)` tuples.
    #[pyo3(signature = (address, group_number, write_priority, change_list, inhibit_delay=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_group<'py>(
        &self,
        py: Python<'py>,
        address: String,
        group_number: u32,
        write_priority: u8,
        change_list: Vec<(Option<PyObjectIdentifier>, Option<u8>, Vec<u8>)>,
        inhibit_delay: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if !(1..=16).contains(&write_priority) {
            return Err(PyValueError::new_err(format!(
                "write_priority must be 1-16, got {write_priority}"
            )));
        }
        let inner = self.inner.clone();
        let cl: Vec<GroupChannelValue> = change_list
            .into_iter()
            .map(|(ch, prio, val)| GroupChannelValue {
                channel: ch.map(|o| o.to_rust()),
                override_priority: prio,
                value: val,
            })
            .collect();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = WriteGroupRequest {
                group_number,
                write_priority,
                change_list: cl,
                inhibit_delay,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.unconfirmed_request(&mac, UnconfirmedServiceChoice::WRITE_GROUP, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Virtual Terminal
    // -----------------------------------------------------------------------

    /// Open a virtual terminal session. Returns the remote session identifier.
    #[pyo3(signature = (address, vt_class))]
    fn vt_open<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vt_class: u32,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = VTOpenRequest { vt_class };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::VT_OPEN, &buf)
                .await
                .map_err(to_py_err)?;
            let ack = VTOpenAck::decode(&resp).map_err(to_py_err)?;
            Ok(ack.remote_vt_session_identifier)
        })
    }

    /// Close one or more virtual terminal sessions.
    #[pyo3(signature = (address, session_ids))]
    fn vt_close<'py>(
        &self,
        py: Python<'py>,
        address: String,
        session_ids: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = VTCloseRequest {
                list_of_remote_vt_session_identifiers: session_ids,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.confirmed_request(&mac, ConfirmedServiceChoice::VT_CLOSE, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send data over a virtual terminal session.
    ///
    /// Returns a dict with optional `all_new_data_accepted` and `accepted_octet_count`.
    #[pyo3(signature = (address, session_id, data, data_flag))]
    fn vt_data<'py>(
        &self,
        py: Python<'py>,
        address: String,
        session_id: u8,
        data: Vec<u8>,
        data_flag: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = VTDataRequest {
                vt_session_identifier: session_id,
                vt_new_data: data,
                vt_data_flag: data_flag,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::VT_DATA, &buf)
                .await
                .map_err(to_py_err)?;
            let ack = VTDataAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("all_new_data_accepted", ack.all_new_data_accepted)?;
                dict.set_item("accepted_octet_count", ack.accepted_octet_count)?;
                Ok(dict.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // Audit
    // -----------------------------------------------------------------------

    /// Send a ConfirmedAuditNotification request (raw service data).
    ///
    /// `service_data` is the pre-encoded AuditNotification-Request payload.
    #[pyo3(signature = (address, service_data))]
    fn confirmed_audit_notification<'py>(
        &self,
        py: Python<'py>,
        address: String,
        service_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.confirmed_request(
                &mac,
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                &service_data,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send an UnconfirmedAuditNotification request (raw service data).
    #[pyo3(signature = (address, service_data))]
    fn unconfirmed_audit_notification<'py>(
        &self,
        py: Python<'py>,
        address: String,
        service_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.unconfirmed_request(
                &mac,
                UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
                &service_data,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send an AuditLogQuery request. Returns raw response bytes.
    #[pyo3(signature = (address, acknowledgment_filter, query_options_raw=vec![]))]
    fn audit_log_query<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledgment_filter: u32,
        query_options_raw: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = AuditLogQueryRequest {
                acknowledgment_filter,
                query_options_raw,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::AUDIT_LOG_QUERY, &buf)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &resp).into_any().unbind()))
        })
    }

    /// Explicitly stop the client.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let arc = {
                let mut guard = inner.lock().await;
                guard.take()
            };
            if let Some(arc) = arc {
                match Arc::try_unwrap(arc) {
                    Ok(mut c) => {
                        c.stop().await.map_err(to_py_err)?;
                    }
                    Err(_arc) => {
                        // Other async operations still hold references;
                        // cleanup will happen when they complete and drop the Arc.
                    }
                }
            }
            Ok(())
        })
    }
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
