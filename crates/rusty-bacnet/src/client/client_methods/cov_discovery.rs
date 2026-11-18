use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // COV
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for an object.
    /// `address` is a local address or a `RoutedTarget`, as for
    /// `read_property` (#2588). A routed device notifies through its
    /// router, so a notification from it has `source_network` and
    /// `source_address` set to the station's network and MAC.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None))]
    fn subscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = monitored_object_identifier.to_rust();
        let process = subscriber_process_identifier;

        let future = async move {
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.subscribe_cov(&mac, process, oid, confirmed, lifetime)
                        .await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .subscribe_cov(&to, process, oid, confirmed, lifetime)
                        .await
                }
            }
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Unsubscribe from COV notifications for an object. `address` is a
    /// local address or a `RoutedTarget`, as for `subscribe_cov`. The
    /// device finds the subscription by the address it saw the request
    /// from, so name the target the subscription named: the same device
    /// reached another way, such as by its own `ip:port` after a routed
    /// subscription, answers with success and keeps the subscription.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier))]
    fn unsubscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = monitored_object_identifier.to_rust();
        let process = subscriber_process_identifier;

        let future = async move {
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.unsubscribe_cov(&mac, process, oid).await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level().unsubscribe_cov(&to, process, oid).await
                }
            }
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Subscribe to COV notifications for one property of an object
    /// (SubscribeCOVProperty, #2588). `address` is a local address or a
    /// `RoutedTarget`, as for `subscribe_cov`. `lifetime` is required, 1 or
    /// more seconds: such a subscription can't be indefinite, so 0 raises
    /// `ValueError` before anything is sent. `array_index` names one element
    /// of an array property; `cov_increment` overrides the object's own
    /// increment for a numeric property. Notifications arrive on
    /// `cov_notifications()`, as for `subscribe_cov`.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, property_identifier, confirmed, lifetime, array_index=None, cov_increment=None))]
    #[allow(clippy::too_many_arguments)]
    fn subscribe_cov_property<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        property_identifier: PyPropertyIdentifier,
        confirmed: bool,
        lifetime: u32,
        array_index: Option<u32>,
        cov_increment: Option<f32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let lifetime = std::num::NonZeroU32::new(lifetime).ok_or_else(|| {
            PyValueError::new_err(
                "lifetime must be 1 or more seconds: a COV-property subscription can't be indefinite",
            )
        })?;
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let subscription = client::CovPropertySubscription {
            subscriber_process_identifier,
            monitored_object_identifier: monitored_object_identifier.to_rust(),
            monitored_property_identifier: property_identifier.to_rust(),
            monitored_property_array_index: array_index,
            confirmed,
            lifetime,
            cov_increment,
        };

        let future = async move {
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.subscribe_cov_property(&mac, subscription).await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .subscribe_cov_property(&to, subscription)
                        .await
                }
            }
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Cancel a single-property COV subscription. Name the target, the
    /// process, the object, the property and its array index the
    /// subscription named: the device finds it by those and the address the
    /// request came from, as for `unsubscribe_cov`.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, property_identifier, array_index=None))]
    fn unsubscribe_cov_property<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        property_identifier: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let address = routed::Address::of(address)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let (process, oid) = (
            subscriber_process_identifier,
            monitored_object_identifier.to_rust(),
        );
        let pid = property_identifier.to_rust();

        let future = async move {
            match address {
                routed::Address::Local(address) => {
                    let mac = routed::Address::local(&address, peers)?;
                    let c = started(&inner).await?;
                    c.unsubscribe_cov_property(&mac, process, oid, pid, array_index)
                        .await
                }
                routed::Address::Routed(target) => {
                    let high = state::high_level(&inner).await?;
                    let to = target.path(&high, &inner.route_misses).await?;
                    high.low_level()
                        .unsubscribe_cov_property(&to, process, oid, pid, array_index)
                        .await
                }
            }
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get an async iterator yielding incoming COV notifications of both
    /// kinds, `CovNotification` and `CovNotificationMultiple`, in arrival
    /// order.
    fn cov_notifications<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let rx = started(&inner).await?.cov_notifications();
            Ok(PyCovNotificationIterator::new(rx))
        })
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Send a WhoHas to find an object by identifier: on this network alone
    /// unless `scope` names every network (`"global"`) or one remote
    /// network by its number, as for `who_is()` (#2596). Give both limits
    /// or neither, each from 0 to 4194303; one alone, `low` above
    /// `high`, or a limit past 4194303 raises `ValueError` before
    /// anything is sent, as does a scope that is none of those.
    #[pyo3(
        signature = (object_id, *, low=None, high=None, scope=None),
        text_signature = "($self, object_id, *, low=None, high=None, scope='local')"
    )]
    fn who_has_by_id<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        low: Option<u32>,
        high: Option<u32>,
        scope: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low, high)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Has")?;
        let inner = self.inner.clone();
        let object = WhoHasObject::Identifier(object_id.to_rust());

        let future = async move {
            let c = started(&inner).await?;
            let scope = scope.on(&c).await?;
            send_who_has(&c, &scope, &object, range).await
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a WhoHas to find an object by name, with the same limits and
    /// `scope` rules as `who_has_by_id()`: this network alone by default.
    #[pyo3(
        signature = (name, *, low=None, high=None, scope=None),
        text_signature = "($self, name, *, low=None, high=None, scope='local')"
    )]
    fn who_has_by_name<'py>(
        &self,
        py: Python<'py>,
        name: String,
        low: Option<u32>,
        high: Option<u32>,
        scope: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let range = device_range(low, high)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Has")?;
        let inner = self.inner.clone();
        let object = WhoHasObject::Name(name);

        let future = async move {
            let c = started(&inner).await?;
            let scope = scope.on(&c).await?;
            send_who_has(&c, &scope, &object, range).await
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get a list of all discovered devices, one per address.
    fn discovered_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            let devices = c.discovered_devices().await;
            Ok(devices
                .into_iter()
                .map(PyDiscoveredDevice::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Look up a discovered device by instance number. Two or more
    /// addresses claiming it raise `BacnetAmbiguousDeviceError`.
    fn get_device<'py>(&self, py: Python<'py>, instance: u32) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            Ok(c.get_device(instance)
                .await
                .map_err(to_py_err)?
                .map(PyDiscoveredDevice::from_rust))
        })
    }

    /// Clear the discovered devices table.
    fn clear_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        let future = async move {
            let c = started(&inner).await?;
            c.clear_devices().await;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Find the VMAC of the B/IPv6 node at `address`, given as `"[ip]:port"`,
    /// and return it as `"bip6:0x0004D2"`, the form the other methods take.
    ///
    /// A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
    /// U.1.3). This asks the node itself with a Virtual-Address-Resolution
    /// sent by unicast, up to three times a second apart (Clause U.5), unless
    /// the client already knows which VMAC is at that address. Needs a
    /// started client with `transport="ipv6"`, else `RuntimeError`. Raises
    /// `BacnetTransportError` when nothing answers within three seconds or
    /// the node refuses.
    fn resolve_vmac<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        crate::py_async::future_into_py(py, async move {
            let peer = parse_bip6_address(&address)?;
            let c = started(&inner).await?;
            let Some(AnyTransport::Bip6(transport)) = c.transport().link() else {
                return Err(PyRuntimeError::new_err(
                    "resolve_vmac needs a client with transport='ipv6'",
                ));
            };
            let vmac = transport
                .resolve_virtual_address(peer)
                .await
                .map_err(to_py_err)?;
            Ok(bacnet_transport::bip6::format_vmac_text(vmac))
        })
    }
}
