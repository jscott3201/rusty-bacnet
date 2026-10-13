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
        ipv6_interface=None,
        *,
        serial_port=None,
        mstp_baud=38400,
        mstp_mac=1,
        mstp_max_master=127,
        mstp_max_info_frames=1,
        mstp_extended_frames=false,
        mstp_echo=false,
        mstp_echo_grace_ms=None,
        mstp_transmit_latency_pad_ms=None,
        mstp_usage_timeout_ms=None,
        mstp_reply_timeout_ms=None,
        sc_device_uuid=None,
        share_port_by_address=false,
        min_request_interval_ms=0,
        ipv6_device_instance=None,
        bbmd=None,
        foreign_device=None
    ))]
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
        serial_port: Option<String>,
        mstp_baud: u32,
        mstp_mac: u8,
        mstp_max_master: u8,
        mstp_max_info_frames: u8,
        mstp_extended_frames: bool,
        mstp_echo: bool,
        mstp_echo_grace_ms: Option<u32>,
        mstp_transmit_latency_pad_ms: Option<u32>,
        mstp_usage_timeout_ms: Option<u32>,
        mstp_reply_timeout_ms: Option<u32>,
        sc_device_uuid: Option<Vec<u8>>,
        share_port_by_address: bool,
        min_request_interval_ms: u64,
        ipv6_device_instance: Option<u32>,
        bbmd: Option<PyRef<'_, crate::bip_options::PyBbmd>>,
        foreign_device: Option<PyRef<'_, crate::bip_options::PyForeignDevice>>,
    ) -> PyResult<Self> {
        crate::bip_options::only_on_bip(share_port_by_address, transport)?;
        let bip_mode = crate::bip_options::bip_mode(transport, bbmd, foreign_device)?;
        let ipv6_device_instance = ipv6_device_instance_for(transport, ipv6_device_instance)?;
        if transport == "sc" {
            crate::tls::required_sc_credentials(
                sc_ca_cert.as_deref(),
                sc_client_cert.as_deref(),
                sc_client_key.as_deref(),
            )
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        let sc_device_uuid = crate::sc_identity::device_uuid(transport, sc_device_uuid)?;
        let min_request_interval_ms = parse_min_request_interval_ms(min_request_interval_ms)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            share_port_by_address,
            bip_mode,
            apdu_timeout_ms,
            sc_hub,
            sc_vmac,
            sc_device_uuid,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            ipv6_interface,
            ipv6_device_instance,
            mstp: crate::mstp_py::MstpOptions {
                serial_port,
                baud: mstp_baud,
                mac: mstp_mac,
                max_master: mstp_max_master,
                max_info_frames: mstp_max_info_frames,
                extended_frames: mstp_extended_frames,
                echo: mstp_echo,
                echo_grace_ms: mstp_echo_grace_ms,
                timing: crate::mstp_py::MstpTiming {
                    transmit_latency_pad_ms: mstp_transmit_latency_pad_ms,
                    usage_timeout_ms: mstp_usage_timeout_ms,
                    reply_timeout_ms: mstp_reply_timeout_ms,
                },
            },
            min_request_interval_ms,
        })
    }

    /// Start the client (called by `async with`).
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let inner = slf.borrow().inner.clone();
        let transport_type = slf.borrow().transport_type.clone();
        let interface_str = slf.borrow().interface.clone();
        let port = slf.borrow().port;
        let broadcast_str = slf.borrow().broadcast_address.clone();
        let share_port_by_address = slf.borrow().share_port_by_address;
        let bip_mode = slf.borrow().bip_mode.clone();
        let timeout_ms = slf.borrow().apdu_timeout_ms;
        let sc_hub = slf.borrow().sc_hub.clone();
        let sc_vmac = slf.borrow().sc_vmac.clone();
        let sc_device_uuid = slf.borrow().sc_device_uuid;
        let sc_ca_cert = slf.borrow().sc_ca_cert.clone();
        let sc_client_cert = slf.borrow().sc_client_cert.clone();
        let sc_client_key = slf.borrow().sc_client_key.clone();
        let sc_heartbeat_interval_ms = slf.borrow().sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = slf.borrow().sc_heartbeat_timeout_ms;
        let ipv6_interface = slf.borrow().ipv6_interface.clone();
        let ipv6_device_instance = slf.borrow().ipv6_device_instance;
        let mstp = slf.borrow().mstp.clone();
        let min_request_interval_ms = slf.borrow().min_request_interval_ms;

        crate::py_async::future_into_py(py, async move {
            let transport: AnyTransport<crate::mstp_py::PySerial> = match transport_type.as_str() {
                "bip" => {
                    let interface: Ipv4Addr = interface_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid interface: {e}")))?;
                    let broadcast: Ipv4Addr = broadcast_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid broadcast: {e}")))?;
                    let mut bip = BipTransport::new(interface, port, broadcast);
                    bip.set_share_port_by_address(share_port_by_address);
                    bip.set_mode(bip_mode);
                    AnyTransport::Bip(Box::new(bip))
                }
                "ipv6" => {
                    let iface_str = ipv6_interface.as_deref().unwrap_or("::");
                    let interface: std::net::Ipv6Addr = iface_str.parse().map_err(|e| {
                        PyRuntimeError::new_err(format!("invalid IPv6 interface: {e}"))
                    })?;
                    AnyTransport::Bip6(Bip6Transport::new(interface, port, ipv6_device_instance))
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

                    let mut sc = bacnet_transport::sc::ScTransport::new(ws, vmac)
                        .with_device_uuid(sc_device_uuid);
                    if let Some(ms) = sc_heartbeat_interval_ms {
                        sc = sc.with_heartbeat_interval_ms(ms);
                    }
                    if let Some(ms) = sc_heartbeat_timeout_ms {
                        sc = sc.with_heartbeat_timeout_ms(ms);
                    }
                    AnyTransport::Sc(Box::new(sc))
                }
                "mstp" => crate::mstp_py::build_mstp_transport(&mstp)?,
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unknown transport: '{other}'. Use 'bip', 'ipv6', 'sc', or 'mstp'"
                    )));
                }
            };

            let c = client::BACnetClient::generic_builder()
                .transport(transport)
                .apdu_timeout_ms(timeout_ms)
                .min_request_interval_ms(min_request_interval_ms)
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
        let future = async move {
            let arc = {
                let mut guard = inner.lock().await;
                guard.take()
            };
            if let Some(arc) = arc {
                // Try to get exclusive ownership for clean stop
                match Arc::try_unwrap(arc) {
                    Ok(mut c) => {
                        if let Err(e) = c.stop().await {
                            #[allow(clippy::print_stderr)]
                            // no logging path in this crate; __aexit__ cannot raise here
                            {
                                eprintln!("BACnetClient stop error in __aexit__: {e}");
                            }
                        }
                    }
                    Err(_arc) => {
                        // Other async operations still hold references;
                        // cleanup will happen when they complete and drop the Arc.
                    }
                }
            }
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
    /// Explicitly stop the client.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let future = async move {
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}

/// The `ipv6_device_instance` keyword: the device instance whose VMAC an ipv6
/// client uses (H.7.2), 0 to 4194303, or `None` for a random VMAC. Any other
/// transport refuses it, and so does an instance out of range, before any
/// I/O.
fn ipv6_device_instance_for(transport: &str, instance: Option<u32>) -> PyResult<Option<u32>> {
    let Some(instance) = instance else {
        return Ok(None);
    };
    if transport != "ipv6" {
        return Err(PyValueError::new_err(
            "ipv6_device_instance applies only to transport='ipv6'",
        ));
    }
    bacnet_transport::bip6::vmac_from_device_instance(instance).map_err(|_| {
        PyValueError::new_err(format!(
            "ipv6_device_instance must be in 0..=4194303, not {instance}"
        ))
    })?;
    Ok(Some(instance))
}
