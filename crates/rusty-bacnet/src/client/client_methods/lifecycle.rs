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
        foreign_device=None,
        multicast=None,
        reconnect=None,
        sc_npdu_per_origin_limit=None,
        sc_failover_hub=None,
        sc_connect_timeout_ms=None,
        sc_control_rate_limit=None,
        router=None
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
        multicast: Option<PyRef<'_, crate::bip_options::PyMulticast>>,
        reconnect: Option<PyRef<'_, crate::sc_reconnect::PyScReconnect>>,
        sc_npdu_per_origin_limit: Option<usize>,
        sc_failover_hub: Option<String>,
        sc_connect_timeout_ms: Option<u64>,
        sc_control_rate_limit: Option<(u32, u32)>,
        router: Option<PyRef<'_, crate::router::PyBACnetRouter>>,
    ) -> PyResult<Self> {
        let router = crate::router::application::stack_router(
            transport,
            router,
            crate::router::application::Role::Client,
            apdu_timeout_ms,
            "apdu_timeout_ms",
        )?;
        let sc_npdu_admission_policy =
            crate::sc_npdu::policy(transport, sc_npdu_per_origin_limit, sc_control_rate_limit)?;
        if reconnect.is_some() && transport != "sc" {
            return Err(PyValueError::new_err(
                "reconnect applies only to transport='sc'",
            ));
        }
        sc_hub_options(
            transport,
            reconnect.is_some(),
            sc_failover_hub.as_deref(),
            sc_connect_timeout_ms,
        )?;
        crate::bip_options::only_on_bip(share_port_by_address, transport)?;
        let crate::bip_options::IpModes {
            bip: bip_mode,
            bip6_foreign,
        } = crate::bip_options::ip_modes(transport, bbmd, foreign_device)?;
        let multicast = crate::bip_options::bip_multicast(
            transport,
            interface,
            port,
            share_port_by_address,
            &bip_mode,
            multicast,
        )?;
        let ipv6_device_instance = ipv6_device_instance_for(transport, ipv6_device_instance)?;
        if bip6_foreign.is_some() && ipv6_device_instance.is_none() {
            // A random VMAC would need the BBMD's help to settle a
            // collision, which the transport doesn't do yet.
            return Err(PyValueError::new_err(
                "foreign_device with transport='ipv6' needs ipv6_device_instance: a B/IPv6 \
                 foreign device uses its instance's VMAC",
            ));
        }
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
            inner: Arc::new(state::ClientState::default()),
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            share_port_by_address,
            bip_mode,
            bip6_foreign,
            multicast,
            apdu_timeout_ms,
            sc_hub,
            sc_vmac,
            sc_device_uuid,
            sc_npdu_admission_policy,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            reconnect: reconnect.map(|options| options.config.clone()),
            sc_failover_hub,
            sc_connect_timeout_ms,
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
            router,
            min_request_interval_ms,
        })
    }

    /// Start the client (called by `async with`).
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        crate::logging_bridge::refresh_at_start(py);
        let self_ref = slf.clone().unbind();
        let inner = slf.borrow().inner.clone();
        let transport_type = slf.borrow().transport_type.clone();
        let interface_str = slf.borrow().interface.clone();
        let port = slf.borrow().port;
        let broadcast_str = slf.borrow().broadcast_address.clone();
        let share_port_by_address = slf.borrow().share_port_by_address;
        let bip_mode = slf.borrow().bip_mode.clone();
        let bip6_foreign = slf.borrow().bip6_foreign.clone();
        let multicast = slf.borrow().multicast;
        let timeout_ms = slf.borrow().apdu_timeout_ms;
        let sc_hub = slf.borrow().sc_hub.clone();
        let sc_vmac = slf.borrow().sc_vmac.clone();
        let sc_device_uuid = slf.borrow().sc_device_uuid;
        let sc_npdu_admission_policy = slf.borrow().sc_npdu_admission_policy;
        let sc_ca_cert = slf.borrow().sc_ca_cert.clone();
        let sc_client_cert = slf.borrow().sc_client_cert.clone();
        let sc_client_key = slf.borrow().sc_client_key.clone();
        let sc_heartbeat_interval_ms = slf.borrow().sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = slf.borrow().sc_heartbeat_timeout_ms;
        let reconnect = slf.borrow().reconnect.clone();
        let sc_failover_hub = slf.borrow().sc_failover_hub.clone();
        let sc_connect_timeout_ms = slf.borrow().sc_connect_timeout_ms;
        let ipv6_interface = slf.borrow().ipv6_interface.clone();
        let ipv6_device_instance = slf.borrow().ipv6_device_instance;
        let mstp = slf.borrow().mstp.clone();
        let min_request_interval_ms = slf.borrow().min_request_interval_ms;
        let router = slf.borrow().router.clone();

        crate::py_async::future_into_py(py, async move {
            let mut slot = state::starting(&inner).await?;
            let c = match transport_type.as_str() {
                "bip" => {
                    let interface: Ipv4Addr = interface_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid interface: {e}")))?;
                    let broadcast: Ipv4Addr = broadcast_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid broadcast: {e}")))?;
                    Box::pin(
                        client::BACnetClient::bip_builder()
                            .interface(interface)
                            .port(port)
                            .broadcast_address(broadcast)
                            .share_port_by_address(share_port_by_address)
                            .mode(bip_mode)
                            .multicast(multicast)
                            .apdu_timeout_ms(timeout_ms)
                            .min_request_interval_ms(min_request_interval_ms)
                            .build_into(),
                    )
                    .await
                }
                "ipv6" => {
                    let iface_str = ipv6_interface.as_deref().unwrap_or("::");
                    let interface: std::net::Ipv6Addr = iface_str.parse().map_err(|e| {
                        PyRuntimeError::new_err(format!("invalid IPv6 interface: {e}"))
                    })?;
                    let mut builder = client::BACnetClient::bip6_builder()
                        .interface(interface)
                        .port(port)
                        .apdu_timeout_ms(timeout_ms)
                        .min_request_interval_ms(min_request_interval_ms);
                    if let Some(instance) = ipv6_device_instance {
                        builder = builder.device_instance(instance);
                    }
                    if let Some(foreign) = bip6_foreign {
                        builder = builder.foreign_device(foreign);
                    }
                    Box::pin(builder.build_into()).await
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

                    let mut builder = client::BACnetClient::sc_builder()
                        .hub_url(&hub_url)
                        .tls_config(tls_config)
                        .vmac(vmac)
                        .device_uuid(sc_device_uuid)
                        .npdu_per_origin_limit(sc_npdu_admission_policy.per_origin_limit)
                        .control_rate_limit(sc_npdu_admission_policy.control_rate)
                        .apdu_timeout_ms(timeout_ms)
                        .min_request_interval_ms(min_request_interval_ms);
                    if let Some(ms) = sc_heartbeat_interval_ms {
                        builder = builder.heartbeat_interval_ms(ms);
                    }
                    if let Some(ms) = sc_heartbeat_timeout_ms {
                        builder = builder.heartbeat_timeout_ms(ms);
                    }
                    if let Some(reconnect) = reconnect {
                        builder = builder.reconnect(reconnect);
                    }
                    if let Some(url) = &sc_failover_hub {
                        builder = builder.failover_hub_url(url);
                    }
                    if let Some(ms) = sc_connect_timeout_ms {
                        builder = builder.connect_timeout_ms(ms);
                    }
                    // Subscribed before the client starts, so the first
                    // connection_events() sees the first Connected (#1908).
                    Box::pin(builder.build_into_with_events())
                        .await
                        .map(|(client, events)| {
                            inner.keep_first_sc_events(events);
                            client
                        })
                }
                "mstp" => {
                    let transport = crate::mstp_py::build_mstp_transport(&mstp)?;
                    Box::pin(
                        client::BACnetClient::generic_builder()
                            .transport(AppTransport::from(transport))
                            .apdu_timeout_ms(timeout_ms)
                            .min_request_interval_ms(min_request_interval_ms)
                            .build(),
                    )
                    .await
                }
                "router" => {
                    let router = router.ok_or_else(|| {
                        PyRuntimeError::new_err("transport='router' needs router=")
                    })?;
                    // Nothing the build checks before starting the port
                    // can fail here: the constructor checked the pacing
                    // interval, the rest are defaults, and every router
                    // port carries a full APDU. So a taken port is a
                    // started one.
                    let port = router.take().await?;
                    Box::pin(
                        client::BACnetClient::generic_builder()
                            .transport(AppTransport::Router(port))
                            .apdu_timeout_ms(timeout_ms)
                            .min_request_interval_ms(min_request_interval_ms)
                            .build(),
                    )
                    .await
                }
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unknown transport: '{other}'. Use 'bip', 'ipv6', 'sc', 'mstp' or \
                         'router'"
                    )));
                }
            }
            .map_err(to_py_err)?;
            *slot = Some(c);
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
        self.stop(py)
    }
    /// Stop admission, drain outstanding operations, and join native teardown.
    ///
    /// Cancelling this Future leaves shutdown running. A later stop joins that
    /// same task. Draining depends on request completion or cancellation.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        crate::py_async::future_into_py(py, crate::unit_result(state::close(self.inner.clone())))
    }
}

/// The `sc_failover_hub` and `sc_connect_timeout_ms` keywords (#1829):
/// SC only, and a failover hub only with `reconnect`, since without
/// reconnection the client would never turn to it or come back. Each is
/// refused with `ValueError` at construction, before any I/O.
fn sc_hub_options(
    transport: &str,
    reconnect: bool,
    failover_hub: Option<&str>,
    connect_timeout_ms: Option<u64>,
) -> PyResult<()> {
    if transport != "sc" && (failover_hub.is_some() || connect_timeout_ms.is_some()) {
        return Err(PyValueError::new_err(
            "sc_failover_hub and sc_connect_timeout_ms apply only to transport='sc'",
        ));
    }
    if failover_hub.is_some() && !reconnect {
        return Err(PyValueError::new_err(
            "sc_failover_hub requires reconnect=ScReconnect(...)",
        ));
    }
    if connect_timeout_ms == Some(0) {
        return Err(PyValueError::new_err(
            "sc_connect_timeout_ms must be at least 1",
        ));
    }
    Ok(())
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
