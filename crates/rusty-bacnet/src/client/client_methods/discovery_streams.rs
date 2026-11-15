//! I-Have delivery and router discovery (#1850): collect the answers to a
//! Who-Has or a Who-Is-Router-To-Network, or follow every I-Have and
//! network report the client receives.

use super::super::*;
use crate::discovery::{PyDiscoveredRouter, PyIHave, PyIHaveIterator, PyNetworkReportIterator};

/// The Who-Has object: an `ObjectIdentifier`, or a name as `str`.
fn who_has_object(object: &Bound<'_, PyAny>) -> PyResult<WhoHasObject> {
    if let Ok(id) = object.extract::<PyObjectIdentifier>() {
        return Ok(WhoHasObject::Identifier(id.to_rust()));
    }
    object
        .extract::<String>()
        .map(WhoHasObject::Name)
        .map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(
                "object must be an ObjectIdentifier or a str name",
            )
        })
}

#[pymethods]
impl BACnetClient {
    /// Send a Who-Has for `object` (an `ObjectIdentifier`, or a name as
    /// `str`) and collect the I-Haves that answer it from that scope for
    /// `timeout_ms`: one per device, object and source address, in the
    /// order they came, at most 4096. No answer is an empty
    /// list. The limits and `scope` follow `who_has_by_id()`'s rules: this
    /// network alone by default (#2596). It holds the started client for the
    /// whole wait, so `stop()` (or leaving `async with`) waits up to
    /// `timeout_ms` for it.
    #[pyo3(
        signature = (object, low_limit=None, high_limit=None, timeout_ms=3000, scope=None),
        text_signature = "($self, object, low_limit=None, high_limit=None, timeout_ms=3000, scope='local')"
    )]
    fn who_has_replies<'py>(
        &self,
        py: Python<'py>,
        object: Bound<'py, PyAny>,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
        timeout_ms: u64,
        scope: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let object = who_has_object(&object)?;
        let range = device_range(low_limit, high_limit)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Has")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            let wait = std::time::Duration::from_millis(timeout_ms);
            let replies = bacnet_ops::raw::who_has_replies(&c, &scope, &object, range, wait)
                .await
                .map_err(scoped_error)?;
            Ok(replies.into_iter().map(PyIHave).collect::<Vec<_>>())
        })
    }

    /// Follow every I-Have this client receives from now on, an answer to
    /// its own Who-Has or not: an async iterator of `IHave`. The client
    /// never waits for it; one more than 256 behind loses the oldest and
    /// counts them in its `missed`. Iteration ends when the client stops.
    fn i_have_events<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let rx = started(&inner).await?.i_have_events();
            Ok(PyIHaveIterator::new(rx))
        })
    }

    /// Broadcast a Who-Is-Router-To-Network for `network` (or every
    /// network) and collect the routers that answer for `timeout_ms`: one
    /// `DiscoveredRouter` per router MAC, in the order first heard. For a
    /// `network`, only the routers that list it. No answer is an empty
    /// list. `network` 0 and 65535 raise `BacnetError`. It holds the
    /// started client for the whole wait, so `stop()` (or leaving
    /// `async with`) waits up to `timeout_ms` for it.
    #[pyo3(signature = (network=None, timeout_ms=3000))]
    fn discover_routers<'py>(
        &self,
        py: Python<'py>,
        network: Option<u16>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let routers = started(&inner)
                .await?
                .discover_routers(network, std::time::Duration::from_millis(timeout_ms))
                .await
                .map_err(to_py_err)?;
            Ok(routers
                .into_iter()
                .map(PyDiscoveredRouter)
                .collect::<Vec<_>>())
        })
    }

    /// Follow the network reports this client receives from now on:
    /// I-Am-Router-To-Network, Network-Number-Is and the rejects of
    /// messages it sent, as an async iterator of `NetworkReport`. The
    /// client never waits for it; one more than 64 behind loses the oldest
    /// and counts them in its `missed`. Iteration ends when the client
    /// stops.
    fn network_reports<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let rx = started(&inner).await?.network_reports();
            Ok(PyNetworkReportIterator::new(rx))
        })
    }
}
