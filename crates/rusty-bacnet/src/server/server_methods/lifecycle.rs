use super::super::*;
use bacnet_transport::port::TransportPort;

#[pymethods]
impl BACnetServer {
    /// Start the server. It will begin responding to BACnet requests.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // Validate the selected sink before TLS/serial/network preparation or
        // ownership transfer. Keep registration stable until it is drained.
        let mut pending = self.lock_pending()?;
        let audit_notification_sink = self.audit_notification_sink;
        if let Some(sink) = audit_notification_sink {
            sink.validate(&pending)?;
        }
        let audit_reporter = self.audit_reporters.clone();
        let mut recipient_input = self
            .audit_recipient
            .lock()
            .map_err(|_| PyRuntimeError::new_err("recipient lock poisoned"))?;
        if let Some(profile) = &audit_reporter {
            for reporter in &profile.reporters {
                audit_configuration::pending_audit_reporter_index(&pending, *reporter)?;
            }
            let recipient = recipient_input.as_ref().ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "target Audit requires configure_audit_recipient before start",
                )
            })?;
            self.validate_audit_recipient_input(recipient)?;
        }
        for object in pending.iter() {
            if object.audit_log_forwarding_internal().is_some() {
                audit_configuration::pending_audit_log_index(&pending, object.object_identifier())?;
            }
        }
        let mut builder = server::BACnetServer::generic_builder()
            .segmentation_supported(self.segmentation_supported)
            .apdu_segment_timeout_ms(self.apdu_segment_timeout_ms);
        if let Some(instance) = self.registered_network_port {
            let oid = bacnet_types::primitives::ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::NETWORK_PORT,
                instance,
            )
            .map_err(to_py_err)?;
            builder = builder.registered_network_port(oid);
        }
        if let Some(profile) = audit_reporter {
            builder = builder.audit_reporters(profile);
        }
        for binding in self.device_bindings.values() {
            builder = builder
                .device_binding(binding.clone())
                .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        }
        // Local TLS failures must leave pending registrations available for retry.
        // Load on each start, not in the constructor, so repaired files are used.
        let sc_tls_config = if self.transport_type == "sc" {
            Some(
                crate::tls::build_client_tls_config(
                    self.sc_ca_cert.as_deref(),
                    self.sc_client_cert.as_deref(),
                    self.sc_client_key.as_deref(),
                )
                .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?,
            )
        } else {
            None
        };
        // MS/TP serial open is synchronous and must succeed before pending
        // registrations are moved into the startup future.
        let mut mstp_transport: Option<AnyTransport<crate::mstp_py::PySerial>> =
            if self.transport_type == "mstp" {
                Some(crate::mstp_py::build_mstp_transport(
                    self.serial_port.as_deref(),
                    self.mstp_baud,
                    self.mstp_mac,
                    self.mstp_max_master,
                    self.mstp_max_info_frames,
                )?)
            } else {
                None
            };

        let inner = self.inner.clone();
        let started = self.started.clone();
        let device_instance = self.device_instance;
        let segmentation_supported = self.segmentation_supported;
        let apdu_segment_timeout_ms = self.apdu_segment_timeout_ms;
        let audit_recipient = recipient_input.take();
        let device_name = self.device_name.clone();
        let transport_type = self.transport_type.clone();
        let interface_str = self.interface.clone();
        let port = self.port;
        let broadcast_str = self.broadcast_address.clone();
        let share_port_by_address = self.share_port_by_address;
        let sc_hub = self.sc_hub.clone();
        let sc_vmac = self.sc_vmac.clone();
        let sc_device_uuid = self.sc_device_uuid;
        let sc_heartbeat_interval_ms = self.sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = self.sc_heartbeat_timeout_ms;
        let ipv6_interface = self.ipv6_interface.clone();
        let dcc_password = self.dcc_password.clone();
        let mutation_policy = self.mutation_policy;
        let dcc_policy = self.dcc_policy;
        let dcc_source_restriction = self.dcc_source_restriction.clone();
        let dcc_disable_rate_limit = self.dcc_disable_rate_limit;
        let reinit_password = self.reinit_password.clone();
        let request_admission_policy = self.request_admission_policy;
        let read_property_multiple_budget = self.read_property_multiple_budget;
        let get_alarm_summary_budget = self.get_alarm_summary_budget;
        let get_enrollment_summary_budget = self.get_enrollment_summary_budget;
        let atomic_read_file_budget = self.atomic_read_file_budget;
        let atomic_write_file_budget = self.atomic_write_file_budget;
        let read_range_budget = self.read_range_budget;
        let get_event_information_budget = self.get_event_information_budget;
        let cov_policy = self.cov_policy.clone();
        let time_sync_policy = self.time_sync_policy.clone();

        let objects: Vec<Box<dyn BACnetObject + Send>> = pending.drain(..).collect();
        // Taken under the pending lock, with the forwarders they belong to.
        let forwarder_save_counters = std::mem::take(
            &mut *self
                .pending_forwarder_save_counters
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?,
        );
        let live_forwarder_save_counters = Arc::clone(&self.forwarder_save_counters);
        self.forwarding_configuration_started
            .store(true, Ordering::Release);
        drop(pending);

        let future = async move {
            let mut db = ObjectDatabase::new();

            // Prepare non-SC transports locally before constructing our Device.
            let prepared_transport: Option<AnyTransport<crate::mstp_py::PySerial>> =
                match transport_type.as_str() {
                    "bip" => {
                        let interface: Ipv4Addr = interface_str.parse().map_err(|e| {
                            PyRuntimeError::new_err(format!("invalid interface: {e}"))
                        })?;
                        let broadcast: Ipv4Addr = broadcast_str.parse().map_err(|e| {
                            PyRuntimeError::new_err(format!("invalid broadcast: {e}"))
                        })?;
                        let mut bip = BipTransport::new(interface, port, broadcast);
                        bip.set_share_port_by_address(share_port_by_address);
                        Some(AnyTransport::Bip(Box::new(bip)))
                    }
                    "ipv6" => {
                        let iface_str = ipv6_interface.as_deref().unwrap_or("::");
                        let interface: std::net::Ipv6Addr = iface_str.parse().map_err(|e| {
                            PyRuntimeError::new_err(format!("invalid IPv6 interface: {e}"))
                        })?;
                        Some(AnyTransport::Bip6(Bip6Transport::new(
                            interface, port, None,
                        )))
                    }
                    // SC can only be constructed after its dial; defer that I/O.
                    "sc" => None,
                    "mstp" => Some(mstp_transport.take().ok_or_else(|| {
                        PyRuntimeError::new_err("MS/TP transport was not prepared")
                    })?),
                    other => {
                        return Err(PyRuntimeError::new_err(format!(
                            "unknown transport: '{other}'. Use 'bip', 'ipv6', 'sc', or 'mstp'"
                        )));
                    }
                };

            let local_capacity =
                match &prepared_transport {
                    Some(transport) => transport.local_receive_apdu_capacity(),
                    // Only the SC branch defers construction. Its receive declaration
                    // is owned by ScTransport and available before opening a socket.
                    None => bacnet_transport::sc::ScTransport::<
                        bacnet_transport::sc_tls::TlsWebSocket,
                    >::LOCAL_RECEIVE_APDU_CAPACITY,
                };

            // Validate the generated Device and entire pending DB before SC I/O.
            let mut device = DeviceObject::new(DeviceConfig {
                segmentation_supported,
                apdu_segment_timeout: apdu_segment_timeout_ms,
                ..generated_device_config(device_instance, device_name, local_capacity)
            })
            .map_err(to_py_err)?;
            if let Some(recipient) = audit_recipient {
                device
                    .provision_audit_recipient(recipient)
                    .map_err(to_py_err)?;
            }

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

            let transport = match prepared_transport {
                Some(transport) => transport,
                None => {
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

                    let tls_config = sc_tls_config
                        .ok_or_else(|| PyRuntimeError::new_err("SC TLS config was not prepared"))?;

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
            };

            let mut builder = builder
                .database(db)
                .mutation_policy(mutation_policy)
                .request_admission_policy(request_admission_policy)
                .read_property_multiple_budget(read_property_multiple_budget)
                .get_alarm_summary_budget(get_alarm_summary_budget)
                .get_enrollment_summary_budget(get_enrollment_summary_budget)
                .atomic_read_file_budget(atomic_read_file_budget)
                .atomic_write_file_budget(atomic_write_file_budget)
                .read_range_budget(read_range_budget)
                .get_event_information_budget(get_event_information_budget)
                .cov_policy(cov_policy)
                .time_sync_policy(time_sync_policy)
                .transport(transport);
            if let Some(sink) = audit_notification_sink {
                builder = builder.audit_notification_sink(sink.object_id);
                if sink.allow_all {
                    builder = builder
                        .audit_notification_authorizer(|_| true)
                        .unconfirmed_audit_notification_authorizer(|_| true);
                }
                // deny_all deliberately retains the Rust fail-closed defaults.
            }
            if let Some(pw) = dcc_password {
                builder = builder.dcc_password(pw);
            }
            builder = builder
                .dcc_policy(dcc_policy)
                .dcc_source_restriction(dcc_source_restriction)
                .dcc_disable_rate_limit(dcc_disable_rate_limit);
            if let Some(pw) = reinit_password {
                builder = builder.reinit_password(pw);
            }
            let srv = builder.build().await.map_err(to_py_err)?;

            // No fallible step once the server runs: it must reach `inner`.
            let mut published = inner.lock().await;
            *live_forwarder_save_counters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = forwarder_save_counters;
            *published = Some(srv);
            drop(published);
            started.store(true, Ordering::Release);
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Stop admitted work and release the owned transport.
    /// Cancellation keeps shutdown available for a later stop to join.
    /// Waits, with no limit, for the saves durable objects have queued, and
    /// warns while storage holds it up (#1363).
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        let future = async move {
            let mut guard = inner.lock().await;
            if let Some(srv) = guard.as_mut() {
                srv.stop().await.map_err(to_py_err)?;
            }
            guard.take();
            started.store(false, Ordering::Release);
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get the retained server instance's last bound address as a string.
    /// This remains a snapshot during interrupted shutdown; successful stop clears it.
    ///
    /// For BIP: `ip:port`, for IPv6: `[ip]:port`, for SC: hex-encoded VMAC.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let transport_type = self.transport_type.clone();
        crate::py_async::future_into_py(py, async move {
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
                "mstp" => {
                    if mac.len() != 1 {
                        return Err(PyRuntimeError::new_err(format!(
                            "unexpected MS/TP MAC length: {}",
                            mac.len()
                        )));
                    }
                    Ok(mac[0].to_string())
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

    /// Read a property from a local object through the server's ReadProperty
    /// evaluator, the one network reads use
    /// ([`read_local`](server::BACnetServer::read_local)).
    ///
    /// A Group's Present_Value is rebuilt from its members, Device instance
    /// 4194303 names the server's Device, and the Device's COV subscription
    /// lists are live. Errors match a network read's: an unknown object or
    /// property raises `BacnetProtocolError`.
    ///
    /// The value is encoded as the server sends it and decoded by the
    /// client's rules, so it has the shape a network `read_property` of the
    /// same property returns. The server lock is held for the read, as for
    /// [`write_property_local`](Self::write_property_local).
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

        crate::py_async::future_into_py(py, async move {
            let value = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.read_local(&oid, pid, array_index)
                    .await
                    .map_err(to_py_err)?
            };
            let mut encoded = bytes::BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(&mut encoded, &value)
                .map_err(to_py_err)?;
            let value =
                crate::types::decode_read_value(oid.object_type(), pid, array_index, &encoded)
                    .map_err(to_py_err)?;
            Ok(value)
        })
    }

    /// Write a property on a local object in the server's database.
    ///
    /// Required `source_object=None` selects the server Device; an identifier
    /// selects an existing local initiator. Tracked writes require a concrete
    /// local Device, which retains correction ownership.
    ///
    /// The value is encoded as a network `WriteProperty` would carry it and
    /// handed to [`write_local_encoded`](server::BACnetServer::write_local_encoded),
    /// which decodes it as the WriteProperty handler does and then takes the
    /// server-owned [`write_local`](server::BACnetServer::write_local) path. So
    /// any value a network write takes works here, including whatever
    /// `read_property` returns, and a local write fires the same post-write
    /// COV and event notifications as a network one. `OBJECT_NAME` writes are
    /// routed through the database name index — a duplicate name is rejected
    /// up front with PROPERTY / `DUPLICATE_NAME` and a successful rename
    /// refreshes the index — so local writes obey the same uniqueness and
    /// lookup invariants as the network handlers.
    ///
    /// Errors are surfaced as `BacnetProtocolError` (with `error_class`/
    /// `error_code`) for parity with the network path — e.g. an unknown object
    /// yields `UNKNOWN_OBJECT` rather than a generic `RuntimeError`, and an
    /// array index on a property that isn't an array yields
    /// `PROPERTY_IS_NOT_AN_ARRAY`.
    ///
    /// The server lock is held for the whole call (including the post-write
    /// COV/event sends), so concurrent Python calls on the same `BACnetServer`
    /// serialize behind a local write. This is deliberate: it prevents a
    /// `stop()` racing the notification sends mid-flight. A confirmed-COV send
    /// to an unresponsive subscriber can therefore stall other Python calls
    /// for up to the COV retry timeout.
    #[pyo3(signature = (object_id, property_id, value, priority=None, array_index=None, *, source_object))]
    fn write_property_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
        source_object: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;
        let source = source_object
            .map_or(bacnet_server::LocalCommandSource::ServerDevice, |object| {
                bacnet_server::LocalCommandSource::Object(object.to_rust())
            });

        let future = async move {
            let mut encoded = bytes::BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(&mut encoded, &prop_value)
                .map_err(to_py_err)?;
            // Hold the server guard for the duration of the call: the write
            // borrows `srv` and runs the post-write COV/event trigger path,
            // so the server must stay alive across the await.
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.write_local_encoded(&oid, pid, array_index, &encoded, priority, source)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Update Present_Value for an application-owned Input, Loop or Life
    /// Safety object.
    ///
    /// This is the narrow application route for finite Analog Input and Loop
    /// REAL values, logical Binary Input Enumerated 0/1 values, in-range
    /// Multi-state Input Unsigned values and Life Safety Point and Zone
    /// Enumerated BACnetLifeSafetyState values. The object implementation owns
    /// validation and Out_Of_Service simulation exclusivity.
    #[pyo3(signature = (object_id, value))]
    fn set_present_value_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        value: PyPropertyValue,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let prop_value = value.inner;

        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_present_value_local(&oid, prop_value)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get the server's current communication state.
    ///
    /// Returns `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION`.
    /// The server refuses the deprecated DISABLE, so it never reports it.
    fn comm_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        crate::py_async::future_into_py(py, async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            Ok(crate::types::PyEnableDisable {
                inner: bacnet_types::enums::EnableDisable::from(srv.comm_state()),
            })
        })
    }
}

// The binding owns this Device; derive its declaration before adding it to DB.
fn generated_device_config(
    instance: u32,
    name: String,
    local_receive_apdu_capacity: u16,
) -> DeviceConfig {
    DeviceConfig {
        instance,
        name,
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        max_apdu_length: server::ServerConfig::default()
            .max_apdu_length
            .min(u32::from(local_receive_apdu_capacity)),
        ..DeviceConfig::default()
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
    #[tokio::test]
    async fn binding_generated_mstp_device_matches_local_480_before_io() {
        let (serial, _peer) = LoopbackSerial::pair();
        let transport: AnyTransport<LoopbackSerial> =
            AnyTransport::Mstp(MstpTransport::new(serial, MstpConfig::default()));
        let config =
            generated_device_config(893, "test".into(), transport.local_receive_apdu_capacity());
        assert_eq!(config.max_apdu_length, 480);
        let device = DeviceObject::new(config).unwrap();
        assert_eq!(
            device
                .read_property(
                    bacnet_types::enums::PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
                    None
                )
                .unwrap(),
            PropertyValue::Unsigned(480)
        );
    }
}
