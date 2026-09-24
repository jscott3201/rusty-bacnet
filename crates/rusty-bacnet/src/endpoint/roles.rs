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

use bacnet_encoding::primitives::decode_application_value;

use crate::errors::to_py_err;
use crate::types::{parse_address, PyObjectIdentifier, PyPropertyIdentifier, PyPropertyValue};

/// Client role: initiates ReadProperty/ReadRange/ReadPropertyMultiple over the owner's single transport.
///
/// Cloned out of a running endpoint via `await endpoint.client()`. No
/// lifecycle methods; survives the owner as a value but fails closed after
/// close.
#[pyclass(name = "EndpointClient", frozen, skip_from_py_object)]
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
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let ack = handle
                .read_property(&mac, oid, pid, array_index)
                .await
                .map_err(to_py_err)?;
            let (value, _) = decode_application_value(&ack.property_value, 0).map_err(to_py_err)?;
            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Read 1–64 explicit references on concrete objects, preserving ordered
    /// values and inline errors. Uses the standalone RPM dictionary conversion.
    #[pyo3(signature = (address, specs))]
    #[allow(clippy::type_complexity)]
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
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
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let ack = handle
                .read_property_multiple(&mac, specs)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| crate::types::rpm_ack_to_py(py, ack))
        })
    }

    /// Read a list/log range; supports all-items, position and sequence forms.
    /// Returns raw item_data bytes and a three-boolean result_flags tuple.
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
        let request = crate::read_range::request(
            &object_id,
            &property_id,
            array_index,
            range_type.as_deref(),
            reference_index,
            reference_seq,
            count,
        )?;
        let handle = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let ack = handle
                .read_range(
                    &mac,
                    request.object_identifier,
                    request.property_identifier,
                    request.property_array_index,
                    request.range,
                )
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| crate::read_range::ack_to_dict(py, ack))
        })
    }

    /// Narrow service scope: the endpoint client initiates ReadProperty, ReadRange and ReadPropertyMultiple.
    ///
    /// Snapshot accessor with no I/O; documents the proven subset
    /// (direct read here; routed variants stay on the Rust handle).
    fn service_scope<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item(
            "initiates",
            vec!["read_property", "read_range", "read_property_multiple"],
        )?;
        dict.set_item("executes", Vec::<String>::new())?;
        Ok(dict.into_any())
    }

    fn __repr__(&self) -> String {
        "EndpointClient(shared-transport read_property read_range read_property_multiple)"
            .to_string()
    }
}

/// Server role: the owner's responder liveness + deferred-reply seam.
///
/// The responder executes ReadProperty automatically; this handle exposes
/// no request callback (poll via `is_session_alive`, never Rust-calls-Python).
/// `suspend_next_reply` arms the one-shot MS/TP deferred-reply path.
#[pyclass(name = "EndpointServer", frozen, skip_from_py_object)]
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
}
