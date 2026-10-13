use super::super::*;
use crate::types::PyVTClass;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // Virtual Terminal
    // -----------------------------------------------------------------------

    /// Open a virtual terminal session. Returns the remote session identifier.
    ///
    /// `vt_class` is a `VTClass`; `local_vt_session_identifier` (0-255) is the
    /// caller's own number for the session, which the peer quotes when it
    /// sends data back.
    #[pyo3(signature = (address, vt_class, local_vt_session_identifier))]
    fn vt_open<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vt_class: PyVTClass,
        local_vt_session_identifier: u8,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = VTOpenRequest {
                vt_class: vt_class.to_rust(),
                local_vt_session_identifier,
            };
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
    ///
    /// `session_ids` must hold at least one identifier, each 0-255. An empty list raises
    /// `ValueError`, or `OverflowError` for an integer that doesn't fit, before anything is
    /// sent.
    #[pyo3(signature = (address, session_ids))]
    fn vt_close<'py>(
        &self,
        py: Python<'py>,
        address: String,
        session_ids: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let req = VTCloseRequest {
            list_of_remote_vt_session_identifiers: session_ids,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;

        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.confirmed_request(&mac, ConfirmedServiceChoice::VT_CLOSE, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send data over a virtual terminal session.
    ///
    /// `data_flag` is the sequence flag (`False` = 0, `True` = 1) that
    /// alternates on each new request for a session.
    ///
    /// Returns a dict with a boolean `all_new_data_accepted` and an
    /// `accepted_octet_count` that is an `int` when the peer accepted only part
    /// of the data and `None` when it accepted all of it.
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
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
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
            crate::py_async::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("all_new_data_accepted", ack.all_new_data_accepted())?;
                dict.set_item("accepted_octet_count", ack.accepted_octet_count())?;
                Ok(dict.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // Audit
    // -----------------------------------------------------------------------

    /// Send a ConfirmedAuditNotification request (raw service data).
    ///
    /// `service_data` must be a complete, pre-encoded Clause 21
    /// AuditNotification-Request payload. This is a raw escape hatch and does
    /// not imply that the bundled server executes the service.
    #[pyo3(signature = (address, service_data))]
    fn confirmed_audit_notification<'py>(
        &self,
        py: Python<'py>,
        address: String,
        service_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a ConfirmedAuditNotification from a validated Python mapping.
    #[pyo3(signature = (address, request))]
    fn confirmed_audit_notification_typed<'py>(
        &self,
        py: Python<'py>,
        address: String,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = audit_notification_request_from_py(request)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.confirmed_audit_notification(&mac, &request)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Send an UnconfirmedAuditNotification request (raw service data).
    ///
    /// `service_data` must be a complete, pre-encoded Clause 21
    /// AuditNotification-Request payload. This is a raw escape hatch and does
    /// not imply that the bundled server executes the service.
    #[pyo3(signature = (address, service_data))]
    fn unconfirmed_audit_notification<'py>(
        &self,
        py: Python<'py>,
        address: String,
        service_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send an UnconfirmedAuditNotification from a validated Python mapping.
    #[pyo3(signature = (address, request))]
    fn unconfirmed_audit_notification_typed<'py>(
        &self,
        py: Python<'py>,
        address: String,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = audit_notification_request_from_py(request)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.unconfirmed_audit_notification(&mac, &request)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Send an AuditLogQuery request (raw service data). Returns raw response bytes.
    ///
    /// `service_data` must be a complete, pre-encoded Clause 21
    /// AuditLogQuery-Request payload. This is a raw escape hatch and does not
    /// imply that the bundled server executes the service.
    #[pyo3(signature = (address, service_data))]
    fn audit_log_query<'py>(
        &self,
        py: Python<'py>,
        address: String,
        service_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::AUDIT_LOG_QUERY, &service_data)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(PyBytes::new(py, &resp).into_any().unbind()))
        })
    }

    /// Send an AuditLogQuery from a validated mapping and return a typed mapping ACK.
    #[pyo3(signature = (address, request))]
    fn audit_log_query_typed<'py>(
        &self,
        py: Python<'py>,
        address: String,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = audit_log_query_request_from_py(request)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c.audit_log_query(&mac, &request).await.map_err(to_py_err)?;
            crate::py_async::attach(|py| audit_log_query_ack_to_py(py, &ack))
        })
    }

    // -----------------------------------------------------------------------
    // Time synchronization
    // -----------------------------------------------------------------------

    /// Send a TimeSynchronization request (unconfirmed) to a remote device.
    ///
    /// `date` is `(year, month, day, day_of_week)`: a real day, the full
    /// year 1900..=2154, month 1-12 and day 1-31, and day_of_week that day's
    /// own weekday, 1=Monday..7=Sunday. `time` is `(hour, minute, second,
    /// hundredths)`, each in range. The request sets a clock, so no field
    /// may be 255 (unspecified) or a pattern value: anything else raises
    /// ValueError before anything is sent (#1501).
    #[pyo3(signature = (address, date, time))]
    fn time_synchronization<'py>(
        &self,
        py: Python<'py>,
        address: String,
        date: (u16, u8, u8, u8),
        time: (u8, u8, u8, u8),
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let (d, t) = crate::types::specific_date_time(date, time)?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.time_synchronization(&mac, d, t)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a UTCTimeSynchronization request (unconfirmed) to a remote device.
    ///
    /// Same arguments, and the same specific date and time, as
    /// `time_synchronization`.
    #[pyo3(signature = (address, date, time))]
    fn utc_time_synchronization<'py>(
        &self,
        py: Python<'py>,
        address: String,
        date: (u16, u8, u8, u8),
        time: (u8, u8, u8, u8),
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let (d, t) = crate::types::specific_date_time(date, time)?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.utc_time_synchronization(&mac, d, t)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Directed / Network WhoIs
    // -----------------------------------------------------------------------

    /// Send a Who-Is to a specific device address (unicast), with the same
    /// limits rule as `who_is()`.
    #[pyo3(signature = (address, low_limit=None, high_limit=None))]
    fn who_is_directed<'py>(
        &self,
        py: Python<'py>,
        address: String,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low_limit, high_limit)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_is_directed(&mac, range).await.map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Auto-routing (_from_device) variants
    // -----------------------------------------------------------------------

    /// Read a property from a device by instance number (auto-routing).
    ///
    /// Looks up the device address from the discovery table.
    #[pyo3(signature = (device_instance, object_id, property_id, array_index=None))]
    fn read_property_from_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_property_from_device(device_instance, oid, pid, array_index)
                .await
                .map_err(to_py_err)?;
            let value = decode_read_ack(&ack).map_err(to_py_err)?;
            Ok(value)
        })
    }

    /// Read multiple properties from a device by instance number (auto-routing).
    #[pyo3(signature = (device_instance, specs))]
    fn read_property_multiple_from_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        specs: Vec<PyReadAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_rpm_specs(specs);

        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_property_multiple_from_device(device_instance, rust_specs)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write a property on a device by instance number (auto-routing).
    /// Priority must be omitted or 1-16; invalid u8 priorities raise ValueError synchronously.
    #[pyo3(signature = (device_instance, object_id, property_id, value, priority=None, array_index=None))]
    fn write_property_to_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        validate_write_priority(priority)?;
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &value.inner).map_err(to_py_err)?;
        let encoded = buf.to_vec();

        let future = async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.write_property_to_device(device_instance, oid, pid, array_index, encoded, priority)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Write multiple properties to a device by instance number (auto-routing).
    #[pyo3(signature = (device_instance, specs))]
    fn write_property_multiple_to_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        specs: Vec<PyWriteAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_wpm_specs(specs)?;

        let future = async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.write_property_multiple_to_device(device_instance, rust_specs)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Manual device table management
    // -----------------------------------------------------------------------

    /// Add a device to the discovery table manually.
    ///
    /// Useful when the device address is known without sending a WhoIs.
    /// Default values are used for max_apdu_length (1476), segmentation (NONE),
    /// and vendor_id (0).
    #[pyo3(signature = (device_instance, address))]
    fn add_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        address: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.add_device(device_instance, &mac)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
