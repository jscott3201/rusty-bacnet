use std::num::NonZeroUsize;

use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Read a property from a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port" (e.g. "192.168.1.100:47808")
    ///         on the local link, or a `RoutedTarget` for a station behind
    ///         a router (#2617)
    ///     object_id: Target object identifier
    ///     property_id: Property to read
    ///     array_index: Optional array index (for array properties)
    ///
    /// Returns: PropertyValue with `.tag`, `.value` accessors
    #[pyo3(signature = (address, object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        crate::py_async::future_into_py(py, async move {
            let ack = match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.read_property(&mac, oid, pid, array_index).await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .read_property(&to, oid, pid, array_index)
                        .await
                }
            }
            .map_err(to_py_err)?;

            let value = decode_read_ack(&ack).map_err(to_py_err)?;

            Ok(value)
        })
    }

    /// Write a property on a remote BACnet device.
    ///
    /// Args:
    ///     address: Target device as "ip:port" on the local link, or a
    ///         `RoutedTarget` for a station behind a router (#2617)
    ///     object_id: Target object identifier
    ///     property_id: Property to write
    ///     value: PropertyValue to write (e.g. `PropertyValue.real(72.5)`)
    ///     priority: Optional priority (1-16); invalid u8 priorities raise ValueError synchronously.
    ///     array_index: Optional array index
    #[pyo3(signature = (address, object_id, property_id, value, priority=None, array_index=None))]
    fn write_property<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        validate_write_priority(priority)?;
        let address = routed::Address::of(address)?;

        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        let future = async move {
            // Encode PropertyValue to application-tagged bytes
            let mut value_buf = BytesMut::new();
            encode_property_value(&mut value_buf, &prop_value).map_err(to_py_err)?;
            let value = value_buf.to_vec();
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.write_property(&mac, oid, pid, array_index, value, priority)
                        .await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .write_property(&to, oid, pid, array_index, value, priority)
                        .await
                }
            }
            .map_err(to_py_err)?;

            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a WhoIs broadcast to discover devices: on this network alone
    /// unless `scope` names every network (`"global"`), one remote
    /// network by its number (#2560), or one station by its address, which
    /// is asked by unicast (#2561).
    ///
    /// Give both limits or neither, each from 0 to 4194303; one alone,
    /// `low` above `high`, or a limit past 4194303 raises
    /// `ValueError` before anything is sent, as does a scope that is none
    /// of those.
    #[pyo3(
        signature = (*, low=None, high=None, scope=None),
        text_signature = "($self, *, low=None, high=None, scope='local')"
    )]
    fn who_is<'py>(
        &self,
        py: Python<'py>,
        low: Option<u32>,
        high: Option<u32>,
        scope: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low, high)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Is")?;
        let inner = self.inner.clone();
        let future = async move {
            let c = started(&inner).await?;
            let scope = scope.on(&c).await?;
            send_who_is(&c, &scope, range).await
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Find devices: one Who-Is, then the devices that answered it within
    /// `wait` seconds (3 unless given), as a list of `Discovered`, one an
    /// address. Only this call's answers: a device the client heard
    /// earlier and that stays quiet now isn't listed
    /// (`discovered_devices()` is the whole table).
    ///
    /// `scope` is where the Who-Is goes, as for `who_is()`: this network
    /// alone by default. `low` and `high` ask only the instances between
    /// them: give both or neither, each from 0 to 4194303. A row is a
    /// `device()` target, and its `duplicate` says when another address
    /// claims the same instance.
    ///
    /// Raised before anything is sent: `ValueError` for one limit alone,
    /// `low` above `high`, a limit past 4194303, a scope that is none of
    /// the forms, or a `wait` that isn't a number of seconds from 0 to
    /// 3600; `TypeError` for a scope of another type.
    #[pyo3(
        signature = (*, scope=None, low=None, high=None, wait=3.0),
        text_signature = "($self, *, scope='local', low=None, high=None, wait=3.0)"
    )]
    fn discover<'py>(
        &self,
        py: Python<'py>,
        scope: Option<Bound<'py, PyAny>>,
        low: Option<u32>,
        high: Option<u32>,
        wait: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low, high)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Is")?;
        let wait = discover_wait(wait)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let high_level = state::high_level(&inner).await?;
            let scope = scope.on(high_level.low_level()).await?;
            let style = high_level.address_style();
            let mut request = bacnet_ops::Discover::from(scope).wait(wait);
            if let Some(range) = range {
                request = request.range(range.low()..=range.high());
            }
            let found = high_level
                .discover(request)
                .await
                .map_err(|error| crate::high_level::ops_err(error, style))?;
            Ok(found
                .into_iter()
                .map(|found| crate::high_level::PyDiscovered::new(found, style))
                .collect::<Vec<_>>())
        })
    }

    // -----------------------------------------------------------------------
    // RPM / WPM
    // -----------------------------------------------------------------------

    /// Read multiple properties from multiple objects in a single request.
    /// `address` is a local address or a `RoutedTarget`, as for
    /// `read_property` (#2617).
    #[pyo3(signature = (address, specs))]
    fn read_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        specs: Vec<PyReadAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let rust_specs = py_to_rpm_specs(specs);

        crate::py_async::future_into_py(py, async move {
            let ack = match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.read_property_multiple(&mac, rust_specs).await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .read_property_multiple(&to, rust_specs)
                        .await
                }
            }
            .map_err(to_py_err)?;
            crate::py_async::attach(|py| rpm_ack_to_py(py, ack))
        })
    }

    /// Write multiple properties to multiple objects in a single request.
    #[pyo3(signature = (address, specs))]
    fn write_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        specs: Vec<PyWriteAccessSpec>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let rust_specs = py_to_wpm_specs(specs)?;

        let future = async move {
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let client = started(&inner).await?;
                    client.write_property_multiple(&mac, rust_specs).await
                }
                routed::Address::Routed(target) => {
                    let client = state::high_level(&inner).await?;
                    let station = target.path(&client, &inner.route_misses).await?;
                    client
                        .low_level()
                        .write_property_multiple(&station, rust_specs)
                        .await
                }
            }
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
            let c = started(&inner).await?;

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
            let c = started(&inner).await?;

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
            let c = started(&inner).await?;

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
