use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Start the server. It will begin responding to BACnet requests.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        let device_instance = self.device_instance;
        let device_name = self.device_name.clone();
        let transport_type = self.transport_type.clone();
        let interface_str = self.interface.clone();
        let port = self.port;
        let broadcast_str = self.broadcast_address.clone();
        let sc_hub = self.sc_hub.clone();
        let sc_vmac = self.sc_vmac.clone();
        let sc_ca_cert = self.sc_ca_cert.clone();
        let sc_client_cert = self.sc_client_cert.clone();
        let sc_client_key = self.sc_client_key.clone();
        let sc_heartbeat_interval_ms = self.sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = self.sc_heartbeat_timeout_ms;
        let ipv6_interface = self.ipv6_interface.clone();
        let dcc_password = self.dcc_password.clone();
        let reinit_password = self.reinit_password.clone();

        // Take pending objects (synchronous, before async block)
        let objects: Vec<Box<dyn BACnetObject + Send>> = {
            let mut guard = self.lock_pending()?;
            guard.drain(..).collect()
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut db = ObjectDatabase::new();

            // Create device object
            let mut device = DeviceObject::new(DeviceConfig {
                instance: device_instance,
                name: device_name,
                vendor_name: "Rusty BACnet".into(),
                vendor_id: 555,
                ..DeviceConfig::default()
            })
            .map_err(to_py_err)?;

            // Collect object identifiers for device object-list
            let dev_oid = device.object_identifier();
            let mut object_list = vec![dev_oid];

            // Move pending objects into the database
            for obj in objects {
                object_list.push(obj.object_identifier());
                db.add(obj).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "duplicate object name: {e}"
                    ))
                })?;
            }

            device.set_object_list(object_list);
            db.add(Box::new(device)).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "duplicate object name: {e}"
                ))
            })?;

            // Build transport based on type
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

            let mut builder = server::BACnetServer::generic_builder()
                .database(db)
                .transport(transport);
            if let Some(pw) = dcc_password {
                builder = builder.dcc_password(pw);
            }
            if let Some(pw) = reinit_password {
                builder = builder.reinit_password(pw);
            }
            let srv = builder.build().await.map_err(to_py_err)?;

            *inner.lock().await = Some(srv);
            started.store(true, Ordering::Release);
            Ok(())
        })
    }

    /// Stop the server.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(mut srv) = guard.take() {
                srv.stop().await.map_err(to_py_err)?;
            }
            started.store(false, Ordering::Release);
            Ok(())
        })
    }

    /// Get the server's local address as a string.
    ///
    /// For BIP: "ip:port", for IPv6: "[ip]:port", for SC: hex-encoded VMAC.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let transport_type = self.transport_type.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            let mac = srv.local_mac();
            match transport_type.as_str() {
                "bip" => {
                    if mac.len() < 6 {
                        return Err(PyRuntimeError::new_err(format!(
                            "unexpected BIP MAC length: {}",
                            mac.len()
                        )));
                    }
                    let ip = Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
                    let port = u16::from_be_bytes([mac[4], mac[5]]);
                    Ok(format!("{ip}:{port}"))
                }
                "ipv6" => {
                    if mac.len() < 18 {
                        return Err(PyRuntimeError::new_err(format!(
                            "unexpected IPv6 MAC length: {}",
                            mac.len()
                        )));
                    }
                    let mut ip_bytes = [0u8; 16];
                    ip_bytes.copy_from_slice(&mac[..16]);
                    let ip = std::net::Ipv6Addr::from(ip_bytes);
                    let port = u16::from_be_bytes([mac[16], mac[17]]);
                    Ok(format!("[{ip}]:{port}"))
                }
                _ => {
                    // SC, Ethernet, or other: hex-encode
                    Ok(mac
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(":"))
                }
            }
        })
    }

    // -----------------------------------------------------------------------
    // Runtime object access
    // -----------------------------------------------------------------------

    /// Read a property from a local object in the server's database.
    #[pyo3(signature = (object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let db_arc = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.database().clone()
            };
            let db = db_arc.read().await;
            let obj = db
                .get(&oid)
                .ok_or_else(|| PyRuntimeError::new_err(format!("object {oid} not found")))?;
            let value = obj.read_property(pid, array_index).map_err(to_py_err)?;
            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Write a property on a local object in the server's database.
    #[pyo3(signature = (object_id, property_id, value, priority=None, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_property_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let db_arc = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.database().clone()
            };
            let mut db = db_arc.write().await;
            let obj = db
                .get_mut(&oid)
                .ok_or_else(|| PyRuntimeError::new_err(format!("object {oid} not found")))?;
            obj.write_property(pid, array_index, prop_value, priority)
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get the server's current communication state.
    ///
    /// Returns 0=Enable, 1=Disable, 2=DisableInitiation.
    fn comm_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            Ok(srv.comm_state())
        })
    }
}
