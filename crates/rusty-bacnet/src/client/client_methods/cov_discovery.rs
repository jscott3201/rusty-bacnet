use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // COV
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None))]
    #[allow(clippy::too_many_arguments)]
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
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
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
        })
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
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.unsubscribe_cov(&mac, subscriber_process_identifier, oid)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get an async iterator yielding incoming COV notifications.
    fn cov_notifications<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let c = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("client not started — use 'async with'"))?;
            let rx = c.cov_notifications();
            Ok(PyCovNotificationIterator::new(rx))
        })
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Send a WhoHas broadcast to find an object by identifier.
    #[pyo3(signature = (object_id, low_limit=None, high_limit=None))]
    fn who_has_by_id<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_has(WhoHasObject::Identifier(oid), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send a WhoHas broadcast to find an object by name.
    #[pyo3(signature = (name, low_limit=None, high_limit=None))]
    fn who_has_by_name<'py>(
        &self,
        py: Python<'py>,
        name: String,
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
            c.who_has(WhoHasObject::Name(name), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get a list of all discovered devices.
    fn discovered_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let devices = c.discovered_devices().await;
            Ok(devices
                .into_iter()
                .map(PyDiscoveredDevice::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Look up a discovered device by instance number.
    fn get_device<'py>(&self, py: Python<'py>, instance: u32) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            Ok(c.get_device(instance)
                .await
                .map(PyDiscoveredDevice::from_rust))
        })
    }

    /// Clear the discovered devices table.
    fn clear_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.clear_devices().await;
            Ok(())
        })
    }
}
