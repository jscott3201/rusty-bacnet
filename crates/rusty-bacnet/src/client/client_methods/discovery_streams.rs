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
    /// `wait` seconds: one per device, object and station, in the order they
    /// came, at most 4096. No answer is an empty
    /// list. The limits and `scope` follow `who_has_by_id()`'s rules: this
    /// network alone by default (#2596). It holds the started client for the
    /// whole wait, so `stop()` (or leaving `async with`) waits up to
    /// `wait` seconds for it.
    #[pyo3(
        signature = (object, *, low=None, high=None, wait=3.0, scope=None),
        text_signature = "($self, object, *, low=None, high=None, wait=3.0, scope='local')"
    )]
    fn who_has_replies<'py>(
        &self,
        py: Python<'py>,
        object: Bound<'py, PyAny>,
        low: Option<u32>,
        high: Option<u32>,
        wait: f64,
        scope: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let object = who_has_object(&object)?;
        let range = device_range(low, high)?;
        let wait = discover_wait(wait)?;
        let scope = discovery_scope(scope.as_ref(), &self.transport_type, "Who-Has")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            let scope = scope.on(&c).await?;
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
    /// network), or with `router` (its address text or MAC bytes on the
    /// local link) ask that one router and keep only its own answers.
    /// Collect for `wait` seconds after the send (an IPv6 socket address
    /// may first need VMAC resolution): one `DiscoveredRouter` per router
    /// MAC, in the order first heard. For a `network`, only the routers
    /// that list it. No answer is an empty list. The asked router answers
    /// with a local broadcast, so a B/IP router on another IP subnet is
    /// heard only when a BBMD forwards its answer here, as a peer or by
    /// foreign-device registration. A `router` that names no one station
    /// raises `ValueError`, as `routing_table` does, with nothing sent;
    /// `network` 0 and 65535 raise `BacnetError`. It holds the started
    /// client through address resolution, the send and collection.
    /// `stop()` waits for the call to finish or be cancelled.
    #[pyo3(signature = (network=None, *, router=None, wait=3.0))]
    fn discover_routers<'py>(
        &self,
        py: Python<'py>,
        network: Option<u16>,
        router: Option<&Bound<'_, PyAny>>,
        wait: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wait = discover_wait(wait)?;
        if let Some(network @ (0 | u16::MAX)) = network {
            return Err(crate::errors::BacnetError::new_err(format!(
                "network {network} is not a network number a router can reach; use 1..=65534"
            )));
        }
        let router = router
            .map(super::routing_table::RouterArg::read)
            .transpose()?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let routers = match router {
                Some(router) => {
                    let mac = router
                        .on(&client, "router needs a router's address on the local link")
                        .await?;
                    // An address that names no one station is an argument
                    // error with nothing sent, as `routing_table` raises it.
                    client
                        .discover_routers_directed(&mac, network, wait)
                        .await
                        .map_err(|error| match error {
                            bacnet_types::error::Error::Transport(io)
                                if io.kind() == std::io::ErrorKind::InvalidInput =>
                            {
                                PyValueError::new_err(io.to_string())
                            }
                            other => to_py_err(other),
                        })?
                }
                None => client
                    .discover_routers(network, wait)
                    .await
                    .map_err(to_py_err)?,
            };
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
