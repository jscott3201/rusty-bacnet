//! Python bindings for the BACnet/SC Hub.
//!
//! The hub is the central relay in a BACnet/SC (Secure Connect) topology.
//! It accepts TLS WebSocket connections from SC nodes and relays messages
//! between them per ASHRAE 135-2020 Annex AB.

use std::sync::Arc;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

use bacnet_transport::sc_hub::ScHub;

use crate::errors::to_py_err;

/// BACnet/SC Hub — relays messages between SC nodes over TLS WebSocket.
///
/// Usage:
/// ```python
/// hub = ScHub(
///     listen="127.0.0.1:0",
///     cert="server.pem",
///     key="server.key",
///     ca_cert="ca.pem",       # optional, enables mTLS
///     vmac=b"\x00\x00\x00\x00\x00\x01",
/// )
/// await hub.start()
/// print(f"Hub listening on {await hub.url()}")
/// # ... hub relays SC traffic ...
/// await hub.stop()
/// ```
#[pyclass(name = "ScHub")]
pub struct PyScHub {
    inner: Arc<Mutex<Option<ScHub>>>,
    listen: String,
    cert: String,
    key: String,
    ca_cert: Option<String>,
    vmac: [u8; 6],
    address: Arc<Mutex<Option<String>>>,
}

#[pymethods]
impl PyScHub {
    /// Create a new SC Hub.
    ///
    /// Args:
    ///     listen: Bind address, e.g. ``"127.0.0.1:0"`` for a random port.
    ///     cert: Path to server certificate PEM file.
    ///     key: Path to server private key PEM file.
    ///     ca_cert: Optional path to CA cert for mTLS (client certificate verification).
    ///     vmac: 6-byte VMAC for the hub itself.
    #[new]
    #[pyo3(signature = (listen, cert, key, vmac, ca_cert=None))]
    fn new(
        listen: &str,
        cert: &str,
        key: &str,
        vmac: Vec<u8>,
        ca_cert: Option<String>,
    ) -> PyResult<Self> {
        if vmac.len() != 6 {
            return Err(PyRuntimeError::new_err("vmac must be exactly 6 bytes"));
        }
        let mut vmac_arr = [0u8; 6];
        vmac_arr.copy_from_slice(&vmac);
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            listen: listen.to_string(),
            cert: cert.to_string(),
            key: key.to_string(),
            ca_cert,
            vmac: vmac_arr,
            address: Arc::new(Mutex::new(None)),
        })
    }

    /// Start the hub. Returns once the hub is listening.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let listen = self.listen.clone();
        let cert = self.cert.clone();
        let key = self.key.clone();
        let ca_cert = self.ca_cert.clone();
        let vmac = self.vmac;
        let address = self.address.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let server_tls = crate::tls::build_server_tls_config(&cert, &key, ca_cert.as_deref())
                .map_err(to_py_err)?;

            let acceptor = TlsAcceptor::from(server_tls);

            let hub = ScHub::start(&listen, acceptor, vmac)
                .await
                .map_err(to_py_err)?;

            let addr = hub
                .local_addr()
                .ok_or_else(|| PyRuntimeError::new_err("hub has no local address"))?;

            *address.lock().await = Some(addr.to_string());
            *inner.lock().await = Some(hub);

            Ok(())
        })
    }

    /// Stop the hub.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if let Some(mut hub) = inner.lock().await.take() {
                hub.stop().await;
            }
            Ok(())
        })
    }

    /// The address the hub is listening on (e.g. ``"127.0.0.1:47900"``).
    ///
    /// Returns ``None`` before ``start()`` is called.
    fn address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let address = self.address.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = address.lock().await;
            Ok(guard.clone())
        })
    }

    /// The ``wss://`` URL for SC clients to connect to.
    ///
    /// Returns ``None`` before ``start()`` is called.
    fn url<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let address = self.address.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = address.lock().await;
            let result: Option<String> = guard
                .as_ref()
                .map(|a| format!("wss://localhost:{}", a.rsplit(':').next().unwrap_or("0")));
            Ok(result)
        })
    }
}
