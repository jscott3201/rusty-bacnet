use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // COV
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None))]
    fn subscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = monitored_object_identifier.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.subscribe_cov(
                &mac,
                subscriber_process_identifier,
                oid,
                confirmed,
                lifetime,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Unsubscribe from COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier))]
    fn unsubscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = monitored_object_identifier.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.unsubscribe_cov(&mac, subscriber_process_identifier, oid)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get an async iterator yielding incoming COV notifications.
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
        signature = (object_id, low=None, high=None, scope=None),
        text_signature = "($self, object_id, low=None, high=None, scope='local')"
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
        signature = (name, low=None, high=None, scope=None),
        text_signature = "($self, name, low=None, high=None, scope='local')"
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
