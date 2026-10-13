//! Two Python role-handle classes above the sibling Rust roles.
//!
//! `EndpointClient` holds a cloned `ClientRoleHandle`; `EndpointServer` holds
//! a cloned `ServerRoleHandle`. Neither exposes lifecycle: start/stop/close
//! exist only on the owning endpoint (`BipEndpoint` / `ScEndpoint` /
//! `MstpEndpoint`). After the owner closes or drops, every call fails closed
//! via `BacnetError` ("endpoint shutdown").
//!
//! No Python callables run under native locks: roles expose no callbacks.
//! Concurrent use is `asyncio.gather` over `read_property` plus
//! `is_session_alive` polling — the same poll pattern as COV iterators,
//! never Rust-calls-Python.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::to_py_err;
use crate::types::{
    parse_address, PyObjectIdentifier, PyPropertyIdentifier, PyPropertyValue, PyReadAccessSpec,
};

/// Client role: initiates ReadProperty/ReadRange/ReadPropertyMultiple/WriteProperty over the owner's single transport.
///
/// Cloned out of a running endpoint via `await endpoint.client()`. No
/// lifecycle methods; survives the owner as a value but fails closed after
/// close.
#[pyclass(
    name = "EndpointClient",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub struct PyEndpointClient {
    inner: bacnet_endpoint::roles::ClientRoleHandle,
}

impl PyEndpointClient {
    pub(crate) fn new(inner: bacnet_endpoint::roles::ClientRoleHandle) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyEndpointClient {
    /// Read a property from a remote device through the shared transport.
    ///
    /// Args:
    ///     address: Target as "ip:port" (BIP), "aa:bb:cc:dd:ee:ff" hex VMAC
    ///         (SC), or "N" / "mstp:N" station (MS/TP).
    ///     object_id: Target object identifier.
    ///     property_id: Property to read.
    ///     array_index: Optional array index.
    ///
    /// Raises `BacnetError` when the owning endpoint is closed, plus the
    /// usual protocol/timeout/reject/abort mapping.
    #[pyo3(signature = (address, object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        crate::py_async::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let ack = handle
                .read_property(&mac, oid, pid, array_index)
                .await
                .map_err(to_py_err)?;
            let value = crate::types::decode_read_ack(&ack).map_err(to_py_err)?;
            Ok(value)
        })
    }

    /// Write a property to a direct IPv4 B/IP peer through the owner's transport.
    /// `commandability` is a required caller assertion, including without a Reporter.
    /// Invalid commandability, priority, framing and APDU size fail synchronously.
    #[pyo3(signature = (address, object_id, property_id, value, priority=None, array_index=None, *, commandability))]
    fn write_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
        commandability: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        use bacnet_endpoint::roles::Commandability;
        let commandability = match commandability {
            "commandable" => Commandability::Commandable,
            "noncommandable" => Commandability::Noncommandable,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "commandability must be 'commandable' or 'noncommandable'",
                ))
            }
        };
        let invalid = |error: bacnet_types::error::Error| {
            pyo3::exceptions::PyValueError::new_err(error.to_string())
        };
        bacnet_services::write_property::validate_priority(priority).map_err(invalid)?;
        let mut encoded = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_property_value(&mut encoded, &value.inner)
            .map_err(invalid)?;
        let request = bacnet_services::write_property::WritePropertyRequest {
            object_identifier: object_id.to_rust(),
            property_identifier: property_id.to_rust(),
            property_array_index: array_index,
            property_value: encoded.to_vec(),
            priority,
        };
        self.inner
            .validate_write_property(&request)
            .map_err(invalid)?;
        let mac = parse_address(&address)?;
        let handle = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            handle
                .write_property(&mac, request, commandability)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Read 1–64 explicit references on concrete objects, preserving ordered
    /// values and inline errors. Uses the standalone RPM dictionary conversion.
    #[pyo3(signature = (address, specs))]
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<PyReadAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let specs = crate::types::py_to_rpm_specs(specs);
        let request = bacnet_client::EndpointReadRequest::Multiple(
            bacnet_services::rpm::ReadPropertyMultipleRequest {
                list_of_read_access_specs: specs,
            },
        );
        request
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let bacnet_client::EndpointReadRequest::Multiple(request) = request else {
            unreachable!()
        };
        let specs = request.list_of_read_access_specs;
        let handle = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let ack = handle
                .read_property_multiple(&mac, specs)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| crate::types::rpm_ack_to_py(py, ack))
        })
    }

    /// Read a list/log range; supports all-items, position, sequence and time
    /// forms, checked strictly or leniently. Returns raw item_data bytes, a
    /// three-boolean result_flags tuple and the rules a lenient read kept.
    #[pyo3(signature = (address, object_id, property_id, array_index=None, range_type=None, reference_index=None, reference_seq=None, count=None, *, reference_time=None, validation="strict"))]
    fn read_range<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
        range_type: Option<String>,
        reference_index: Option<u64>,
        reference_seq: Option<u64>,
        count: Option<i16>,
        reference_time: Option<Bound<'py, PyAny>>,
        validation: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = crate::read_range::request(
            &object_id,
            &property_id,
            array_index,
            range_type.as_deref(),
            reference_index,
            reference_seq,
            count,
            reference_time.as_ref(),
        )?;
        let validation = crate::read_range::validation(validation)?;
        let handle = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let reply = handle
                .read_range_with(&mac, &request, validation)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| crate::read_range::reply_to_dict(py, reply))
        })
    }

    /// Read one page of a log's Log_Buffer from `cursor` (None for the
    /// oldest record), up to `page_size` records; each request is audited.
    #[pyo3(signature = (address, object_id, cursor=None, page_size=100))]
    fn read_log_page<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        cursor: Option<Bound<'py, PyAny>>,
        #[pyo3(from_py_with = crate::log_records::page_size_arg)] page_size: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (log, cursor) = crate::log_records::page_request(&object_id, cursor.as_ref())?;
        let handle = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let page = handle
                .read_log_page(&mac, log, cursor, page_size)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| crate::log_records::page_to_py(py, page))
        })
    }

    /// Narrow service scope: the endpoint client initiates ReadProperty, ReadRange, ReadPropertyMultiple and direct B/IP WriteProperty.
    ///
    /// Snapshot accessor with no I/O; documents the proven subset
    /// (direct read here; routed variants stay on the Rust handle).
    fn service_scope<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item(
            "initiates",
            vec![
                "read_property",
                "read_range",
                "read_property_multiple",
                "write_property",
            ],
        )?;
        dict.set_item("executes", Vec::<String>::new())?;
        Ok(dict.into_any())
    }

    fn __repr__(&self) -> String {
        "EndpointClient(shared-transport read_property read_range read_property_multiple write_property)"
            .to_string()
    }

    /// Refuses `copy` and `pickle`: a handle on a live endpoint. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Server role: the owner's responder liveness + deferred-reply seam.
///
/// The responder executes ReadProperty automatically; this handle exposes
/// no request callback (poll via `is_session_alive`, never Rust-calls-Python).
/// `suspend_next_reply` arms the one-shot MS/TP deferred-reply path.
#[pyclass(
    name = "EndpointServer",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub struct PyEndpointServer {
    inner: bacnet_endpoint::roles::ServerRoleHandle,
}

impl PyEndpointServer {
    pub(crate) fn new(inner: bacnet_endpoint::roles::ServerRoleHandle) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyEndpointServer {
    /// Returns true while the owning endpoint is alive and open.
    ///
    /// Cloned handles survive the owner as values but report false and fail
    /// closed after stop/drop.
    fn is_session_alive(&self) -> bool {
        self.inner.is_session_alive()
    }

    /// Arms one-shot deferred-reply suspension (MS/TP ReplyPostponed wiring).
    ///
    /// The next `reply_tx`-bearing inbound request answers token-owned via
    /// egress instead of promptly. Fails closed with `BacnetError` after
    /// owner close. Broadcasts never consume the arm.
    fn suspend_next_reply(&self) -> PyResult<()> {
        self.inner.suspend_next_reply().map_err(to_py_err)
    }

    /// Narrow service scope: the endpoint server executes ReadProperty only.
    fn service_scope<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item("initiates", Vec::<String>::new())?;
        dict.set_item("executes", vec!["read_property"])?;
        Ok(dict.into_any())
    }

    fn __repr__(&self) -> String {
        "EndpointServer(read_property responder)".to_string()
    }

    /// Refuses `copy` and `pickle`: a handle on a live endpoint. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
