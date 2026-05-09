use super::super::*;

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
