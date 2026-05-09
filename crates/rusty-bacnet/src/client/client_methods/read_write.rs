use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Read a property from a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port" (e.g. "192.168.1.100:47808")
    ///     object_id: Target object identifier
    ///     property_id: Property to read
    ///     array_index: Optional array index (for array properties)
    ///
    /// Returns: PropertyValue with `.tag`, `.value` accessors
    #[pyo3(signature = (address, object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed

            let ack = c
                .read_property(&mac, oid, pid, array_index)
                .await
                .map_err(to_py_err)?;

            // Decode application-tagged value bytes → PropertyValue
            let (value, _) = decode_application_value(&ack.property_value, 0).map_err(to_py_err)?;

            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Write a property on a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port"
    ///     object_id: Target object identifier
    ///     property_id: Property to write
    ///     value: PropertyValue to write (e.g. `PropertyValue.real(72.5)`)
    ///     priority: Optional priority (1-16, for commandable properties)
    ///     array_index: Optional array index
    #[pyo3(signature = (address, object_id, property_id, value, priority=None, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_property<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Some(p) = priority {
            if !(1..=16).contains(&p) {
                return Err(PyValueError::new_err(format!(
                    "priority must be 1-16, got {p}"
                )));
            }
        }

        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed

            // Encode PropertyValue to application-tagged bytes
            let mut value_buf = BytesMut::new();
            encode_property_value(&mut value_buf, &prop_value).map_err(to_py_err)?;

            c.write_property(&mac, oid, pid, array_index, value_buf.to_vec(), priority)
                .await
                .map_err(to_py_err)?;

            Ok(())
        })
    }

    /// Send a WhoIs broadcast to discover devices.
    #[pyo3(signature = (low_limit=None, high_limit=None))]
    fn who_is<'py>(
        &self,
        py: Python<'py>,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed
            c.who_is(low_limit, high_limit).await.map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Convenience: send WhoIs, wait for `timeout_ms` (default 3000), return discovered devices.
    ///
    /// Combines `who_is()` + `asyncio.sleep()` + `discovered_devices()` in one call.
    #[pyo3(signature = (timeout_ms=3000, low_limit=None, high_limit=None))]
    fn discover<'py>(
        &self,
        py: Python<'py>,
        timeout_ms: u64,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_is(low_limit, high_limit).await.map_err(to_py_err)?;
            tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
            let devices = c.discovered_devices().await;
            Ok(devices
                .into_iter()
                .map(PyDiscoveredDevice::from_rust)
                .collect::<Vec<_>>())
        })
    }

    // -----------------------------------------------------------------------
    // RPM / WPM
    // -----------------------------------------------------------------------

    /// Read multiple properties from multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    #[allow(clippy::type_complexity)]
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs = py_to_rpm_specs(specs);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_property_multiple(&mac, rust_specs)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write multiple properties to multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    #[allow(clippy::type_complexity)]
    fn write_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
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
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.write_property_multiple(&mac, rust_specs)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Multi-device batch operations
    // -----------------------------------------------------------------------

    /// Read a property from multiple discovered devices concurrently.
    ///
    /// Args:
    ///     requests: List of (device_instance, object_id, property_id, array_index) tuples
    ///     max_concurrent: Max concurrent requests (default 32)
    ///
    /// Returns: List of dicts with 'device_instance', 'value' (PropertyValue or None),
    ///          'error' (str or None)
    #[pyo3(signature = (requests, max_concurrent=None))]
    fn read_property_from_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<(u32, PyObjectIdentifier, PyPropertyIdentifier, Option<u32>)>,
        max_concurrent: Option<usize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_requests: Vec<_> = requests
            .into_iter()
            .map(
                |(device_instance, oid, pid, idx)| bacnet_client::client::DeviceReadRequest {
                    device_instance,
                    object_identifier: oid.to_rust(),
                    property_identifier: pid.to_rust(),
                    property_array_index: idx,
                },
            )
            .collect();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .read_property_from_devices(rust_requests, max_concurrent)
                .await;

            Python::attach(|py| {
                let py_results: Vec<_> = results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("device_instance", r.device_instance).unwrap();
                        match r.result {
                            Ok(ack) => match decode_application_value(&ack.property_value, 0) {
                                Ok((value, _)) => {
                                    dict.set_item("value", PyPropertyValue::from_rust(value))
                                        .unwrap();
                                    dict.set_item("error", py.None()).unwrap();
                                }
                                Err(e) => {
                                    dict.set_item("value", py.None()).unwrap();
                                    dict.set_item("error", e.to_string()).unwrap();
                                }
                            },
                            Err(e) => {
                                dict.set_item("value", py.None()).unwrap();
                                dict.set_item("error", e.to_string()).unwrap();
                            }
                        }
                        dict.into_any().unbind()
                    })
                    .collect();
                Ok(py_results)
            })
        })
    }

    /// Read multiple properties from multiple devices concurrently (RPM batch).
    ///
    /// Args:
    ///     requests: List of (device_instance, [(object_id, [(property_id, array_index)])]) tuples
    ///     max_concurrent: Max concurrent requests (default 32)
    ///
    /// Returns: List of dicts with 'device_instance', 'results' (list or None), 'error' (str or None)
    #[pyo3(signature = (requests, max_concurrent=None))]
    #[allow(clippy::type_complexity)]
    fn read_property_multiple_from_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<(
            u32,
            Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
        )>,
        max_concurrent: Option<usize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        use bacnet_services::common::PropertyReference;

        let inner = self.inner.clone();
        let rust_requests: Vec<_> = requests
            .into_iter()
            .map(|(device_instance, specs)| {
                let rust_specs = specs
                    .into_iter()
                    .map(
                        |(oid, props)| bacnet_services::rpm::ReadAccessSpecification {
                            object_identifier: oid.to_rust(),
                            list_of_property_references: props
                                .into_iter()
                                .map(|(pid, idx)| PropertyReference {
                                    property_identifier: pid.to_rust(),
                                    property_array_index: idx,
                                })
                                .collect(),
                        },
                    )
                    .collect();
                bacnet_client::client::DeviceRpmRequest {
                    device_instance,
                    specs: rust_specs,
                }
            })
            .collect();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .read_property_multiple_from_devices(rust_requests, max_concurrent)
                .await;

            Python::attach(|py| {
                let py_results: Vec<_> = results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("device_instance", r.device_instance).unwrap();
                        match r.result {
                            Ok(ack) => match rpm_ack_to_py(py, ack) {
                                Ok(rpm_result) => {
                                    dict.set_item("results", rpm_result).unwrap();
                                    dict.set_item("error", py.None()).unwrap();
                                }
                                Err(e) => {
                                    dict.set_item("results", py.None()).unwrap();
                                    dict.set_item("error", e.to_string()).unwrap();
                                }
                            },
                            Err(e) => {
                                dict.set_item("results", py.None()).unwrap();
                                dict.set_item("error", e.to_string()).unwrap();
                            }
                        }
                        dict.into_any().unbind()
                    })
                    .collect();
                Ok(py_results)
            })
        })
    }

    /// Write a property on multiple devices concurrently.
    ///
    /// Args:
    ///     requests: List of (device_instance, object_id, property_id, value, priority, array_index)
    ///     max_concurrent: Max concurrent requests (default 32)
    ///
    /// Returns: List of dicts with 'device_instance', 'error' (str or None)
    #[pyo3(signature = (requests, max_concurrent=None))]
    #[allow(clippy::type_complexity)]
    fn write_property_to_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<(
            u32,
            PyObjectIdentifier,
            PyPropertyIdentifier,
            PyPropertyValue,
            Option<u8>,
            Option<u32>,
        )>,
        max_concurrent: Option<usize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_requests: Result<Vec<_>, PyErr> = requests
            .into_iter()
            .map(
                |(device_instance, oid, pid, value, priority, array_index)| {
                    let mut value_buf = BytesMut::new();
                    encode_property_value(&mut value_buf, &value.inner).map_err(to_py_err)?;
                    Ok(bacnet_client::client::DeviceWriteRequest {
                        device_instance,
                        object_identifier: oid.to_rust(),
                        property_identifier: pid.to_rust(),
                        property_array_index: array_index,
                        property_value: value_buf.to_vec(),
                        priority,
                    })
                },
            )
            .collect();
        let rust_requests = rust_requests?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .write_property_to_devices(rust_requests, max_concurrent)
                .await;

            Python::attach(|py| {
                let py_results: Vec<_> = results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("device_instance", r.device_instance).unwrap();
                        match r.result {
                            Ok(()) => {
                                dict.set_item("error", py.None()).unwrap();
                            }
                            Err(e) => {
                                dict.set_item("error", e.to_string()).unwrap();
                            }
                        }
                        dict.into_any().unbind()
                    })
                    .collect();
                Ok(py_results)
            })
        })
    }
}
