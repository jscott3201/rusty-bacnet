use std::num::NonZeroUsize;

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
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
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

            let value = decode_read_ack(&ack).map_err(to_py_err)?;

            Ok(value)
        })
    }

    /// Write a property on a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port"
    ///     object_id: Target object identifier
    ///     property_id: Property to write
    ///     value: PropertyValue to write (e.g. `PropertyValue.real(72.5)`)
    ///     priority: Optional priority (1-16); invalid u8 priorities raise ValueError synchronously.
    ///     array_index: Optional array index
    #[pyo3(signature = (address, object_id, property_id, value, priority=None, array_index=None))]
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
        validate_write_priority(priority)?;

        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        let future = async move {
            let mac = peers.parse(&address)?;
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a WhoIs broadcast to discover devices.
    ///
    /// Give both limits or neither, each from 0 to 4194303; one alone,
    /// `low_limit` above `high_limit`, or a limit past 4194303 raises
    /// `ValueError` before anything is sent.
    #[pyo3(signature = (low_limit=None, high_limit=None))]
    fn who_is<'py>(
        &self,
        py: Python<'py>,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low_limit, high_limit)?;
        let inner = self.inner.clone();
        let future = async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            // Mutex released here — concurrent calls can proceed
            c.who_is(range).await.map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Convenience: send WhoIs, wait for `timeout_ms` (default 3000), return discovered devices.
    ///
    /// Combines `who_is()` + `asyncio.sleep()` + `discovered_devices()` in one call,
    /// with the same limits rule as `who_is()`.
    #[pyo3(signature = (timeout_ms=3000, low_limit=None, high_limit=None))]
    fn discover<'py>(
        &self,
        py: Python<'py>,
        timeout_ms: u64,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low_limit, high_limit)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_is(range).await.map_err(to_py_err)?;
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
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<PyReadAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let rust_specs = py_to_rpm_specs(specs);

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
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
            crate::py_async::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write multiple properties to multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    fn write_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        specs: Vec<PyWriteAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let rust_specs = py_to_wpm_specs(specs)?;

        let future = async move {
            let mac = peers.parse(&address)?;
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Multi-device batch operations
    // -----------------------------------------------------------------------

    /// Read a property from multiple discovered devices concurrently.
    ///
    /// Args:
    ///     requests: List of (device_instance, object_id, property_id, array_index) tuples
    ///     max_concurrent: Positive native-sized integer (None uses 32). Zero raises
    ///         ValueError synchronously; integers outside usize raise OverflowError.
    ///
    /// Returns: An asyncio Future yielding a list of dicts with 'request_index' (original zero-based occurrence),
    ///          'device_instance', 'value' (PropertyValue or None),
    ///          'error' (BacnetError instance or None). Results use completion order.
    /// Python result-construction failures raise from the whole call; cancellation
    /// returns no partial result list.
    #[pyo3(signature = (requests, max_concurrent=None))]
    fn read_property_from_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<(u32, PyObjectIdentifier, PyPropertyIdentifier, Option<u32>)>,
        max_concurrent: Option<NonZeroUsize>,
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

        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .read_property_from_devices(rust_requests, max_concurrent)
                .await;

            crate::py_async::attach(|py| {
                results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("request_index", r.request_index)?;
                        dict.set_item("device_instance", r.device_instance)?;
                        match r.result {
                            Ok(ack) => match decode_read_ack(&ack) {
                                Ok(value) => {
                                    dict.set_item("value", value)?;
                                    dict.set_item("error", py.None())?;
                                }
                                Err(e) => {
                                    dict.set_item("value", py.None())?;
                                    dict.set_item("error", to_py_err(e).value(py))?;
                                }
                            },
                            Err(e) => {
                                dict.set_item("value", py.None())?;
                                dict.set_item("error", to_py_err(e).value(py))?;
                            }
                        }
                        Ok(dict.into_any().unbind())
                    })
                    .collect::<PyResult<Vec<_>>>()
            })
        })
    }

    /// Read multiple properties from multiple devices concurrently (RPM batch).
    ///
    /// Args:
    ///     requests: List of (device_instance, [(object_id, [(property_id, array_index)])]) tuples
    ///     max_concurrent: Positive native-sized integer (None uses 32). Zero raises
    ///         ValueError synchronously; integers outside usize raise OverflowError.
    ///
    /// Returns: An asyncio Future yielding a list of dicts with 'request_index' (original zero-based occurrence),
    ///          'device_instance', 'results' (list or None), 'error' (BacnetError instance or None). Results use completion order.
    /// Python result-construction failures raise from the whole call; cancellation
    /// returns no partial result list.
    #[pyo3(signature = (requests, max_concurrent=None))]
    fn read_property_multiple_from_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<(u32, Vec<PyReadAccessSpec>)>,
        max_concurrent: Option<NonZeroUsize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        use bacnet_types::constructed::PropertyReference;

        let inner = self.inner.clone();
        let rust_requests: Vec<_> = requests
            .into_iter()
            .map(|(device_instance, specs)| {
                let rust_specs = specs
                    .into_iter()
                    .map(
                        |(oid, props)| bacnet_types::constructed::ReadAccessSpecification {
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

        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .read_property_multiple_from_devices(rust_requests, max_concurrent)
                .await;

            crate::py_async::attach(|py| {
                results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("request_index", r.request_index)?;
                        dict.set_item("device_instance", r.device_instance)?;
                        match r.result {
                            Ok(ack) => {
                                // Python construction failures remain call-level exceptions;
                                // nested BACnet property errors/raw bytes remain RPM data.
                                dict.set_item("results", rpm_ack_to_py(py, ack)?)?;
                                dict.set_item("error", py.None())?;
                            }
                            Err(e) => {
                                dict.set_item("results", py.None())?;
                                dict.set_item("error", to_py_err(e).value(py))?;
                            }
                        }
                        Ok(dict.into_any().unbind())
                    })
                    .collect::<PyResult<Vec<_>>>()
            })
        })
    }

    /// Write a property on multiple devices concurrently.
    ///
    /// Every priority is validated synchronously before any batch dispatch.
    ///
    /// Args:
    ///     requests: List of (device_instance, object_id, property_id, value, priority, array_index)
    ///     max_concurrent: Positive native-sized integer (None uses 32). Zero raises
    ///         ValueError synchronously; integers outside usize raise OverflowError.
    ///
    /// Returns: An asyncio Future yielding a list of dicts with 'request_index' (original zero-based occurrence),
    ///          'device_instance', 'error' (BacnetError instance or None). Results use completion order.
    /// Python result-construction failures raise from the whole call; cancellation
    /// returns no partial result list.
    #[pyo3(signature = (requests, max_concurrent=None))]
    fn write_property_to_devices<'py>(
        &self,
        py: Python<'py>,
        requests: Vec<PyDeviceWrite>,
        max_concurrent: Option<NonZeroUsize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_requests: Result<Vec<_>, PyErr> = requests
            .into_iter()
            .map(
                |(device_instance, oid, pid, value, priority, array_index)| {
                    validate_write_priority(priority)?;
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

        crate::py_async::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };

            let results = c
                .write_property_to_devices(rust_requests, max_concurrent)
                .await;

            crate::py_async::attach(|py| {
                results
                    .into_iter()
                    .map(|r| {
                        let dict = PyDict::new(py);
                        dict.set_item("request_index", r.request_index)?;
                        dict.set_item("device_instance", r.device_instance)?;
                        match r.result {
                            Ok(()) => {
                                dict.set_item("error", py.None())?;
                            }
                            Err(e) => {
                                dict.set_item("error", to_py_err(e).value(py))?;
                            }
                        }
                        Ok(dict.into_any().unbind())
                    })
                    .collect::<PyResult<Vec<_>>>()
            })
        })
    }
}
