use super::super::*;

#[pymethods]
impl BACnetClient {
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

    // -----------------------------------------------------------------------
    // Time synchronization
    // -----------------------------------------------------------------------

    /// Send a TimeSynchronization request (unconfirmed) to a remote device.
    ///
    /// `date` is `(year, month, day, day_of_week)` where year is the full year
    /// (e.g. 2026), month 1-12, day 1-31, day_of_week 1=Monday..7=Sunday
    /// (or 255 for unspecified).
    /// `time` is `(hour, minute, second, hundredths)`.
    #[pyo3(signature = (address, date, time))]
    fn time_synchronization<'py>(
        &self,
        py: Python<'py>,
        address: String,
        date: (u16, u8, u8, u8),
        time: (u8, u8, u8, u8),
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let d = bacnet_types::primitives::Date {
            year: date.0.saturating_sub(1900) as u8,
            month: date.1,
            day: date.2,
            day_of_week: date.3,
        };
        let t = bacnet_types::primitives::Time {
            hour: time.0,
            minute: time.1,
            second: time.2,
            hundredths: time.3,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
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
        })
    }

    /// Send a UTCTimeSynchronization request (unconfirmed) to a remote device.
    ///
    /// Same argument format as `time_synchronization`.
    #[pyo3(signature = (address, date, time))]
    fn utc_time_synchronization<'py>(
        &self,
        py: Python<'py>,
        address: String,
        date: (u16, u8, u8, u8),
        time: (u8, u8, u8, u8),
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let d = bacnet_types::primitives::Date {
            year: date.0.saturating_sub(1900) as u8,
            month: date.1,
            day: date.2,
            day_of_week: date.3,
        };
        let t = bacnet_types::primitives::Time {
            hour: time.0,
            minute: time.1,
            second: time.2,
            hundredths: time.3,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
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
        })
    }

    // -----------------------------------------------------------------------
    // Directed / Network WhoIs
    // -----------------------------------------------------------------------

    /// Send a Who-Is to a specific device address (unicast).
    #[pyo3(signature = (address, low_limit=None, high_limit=None))]
    fn who_is_directed<'py>(
        &self,
        py: Python<'py>,
        address: String,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
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
            c.who_is_directed(&mac, low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
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

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
            let (value, _) = decode_application_value(&ack.property_value, 0).map_err(to_py_err)?;
            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Read multiple properties from a device by instance number (auto-routing).
    #[pyo3(signature = (device_instance, specs))]
    #[allow(clippy::type_complexity)]
    fn read_property_multiple_from_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
        specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_rpm_specs(specs);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
            Python::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write a property on a device by instance number (auto-routing).
    #[pyo3(signature = (device_instance, object_id, property_id, value, priority=None, array_index=None))]
    #[allow(clippy::too_many_arguments)]
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
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let mut buf = BytesMut::new();
        let _ = encode_property_value(&mut buf, &value.inner);
        let encoded = buf.to_vec();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
        })
    }

    /// Write multiple properties to a device by instance number (auto-routing).
    #[pyo3(signature = (device_instance, specs))]
    #[allow(clippy::type_complexity)]
    fn write_property_multiple_to_device<'py>(
        &self,
        py: Python<'py>,
        device_instance: u32,
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
        })
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
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
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
        })
    }
}
