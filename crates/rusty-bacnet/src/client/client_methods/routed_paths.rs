//! Routed-path size limits (#2667): set and clear a path's limit by hand,
//! list the limits the client holds, and the two counts of too-long
//! rejects it did not believe.
//!
//! A path is a router on the local link and a destination network. Since
//! #2665 the client does not believe a router's too-long reject for a
//! request of 228 NPDU octets or fewer, the least every router must carry
//! (Table 6-1), so a router that really carries less shows as timeouts
//! until its path's limit is set here.

use bacnet_ops::target::AddressStyle;
use bacnet_types::error::Error;
use pyo3::exceptions::PyTypeError;
use pyo3::types::PyBool;

use super::super::*;
use super::routing_table::RouterArg;
use crate::routed_paths::PyRoutedPathLimit;

/// A path's network as the caller passed it: an `int`, and not a `bool`,
/// as `RoutedTarget` takes it. The client checks the range.
fn network_arg(network: &Bound<'_, PyAny>) -> PyResult<u16> {
    if network.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err("network must be an int, not bool"));
    }
    network.extract()
}

/// A refused argument (a router MAC of the wrong length, a network outside
/// 1 to 65534, a limit too small to carry a request) is a ValueError; the
/// rest as every call raises it.
fn path_err(error: Error) -> PyErr {
    match error {
        Error::Encoding(message) => PyValueError::new_err(message),
        other => to_py_err(other),
    }
}

#[pymethods]
impl BACnetClient {
    /// Set the size limit, in NPDU octets, of the routed path through
    /// `router` to `network`, replacing any limit set or learned before.
    /// Requests to stations on that network through that router are then
    /// sized to it, and segmented or refused as any request too large for
    /// its path is.
    ///
    /// `router` is the router's address on the local link, as
    /// `routing_table` takes it: `bytes` such as a `DiscoveredRouter`'s
    /// `mac`, or a string in the binding's address forms. `network` is 1 to
    /// 65534. A limit under 228 octets is taken, for an older router that
    /// carries less than the standard asks; one that could carry no request
    /// at all (under 66 on B/IP, 61 on MS/TP) raises ValueError, as do a bad
    /// router or network. The call waits for a request active on the path.
    /// Sends nothing. Raises RuntimeError before start and after stop.
    fn configure_routed_path_max_npdu<'py>(
        &self,
        py: Python<'py>,
        router: Bound<'py, PyAny>,
        network: Bound<'py, PyAny>,
        max_npdu: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let router = RouterArg::read(&router)?;
        let network = network_arg(&network)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let mac = router.on(&client).await?;
            client
                .configure_routed_path_max_npdu(&mac, network, max_npdu)
                .await
                .map_err(path_err)
        })
    }

    /// Put the routed path through `router` to `network` back on the
    /// default, 228 octets, dropping the limit set or learned for it.
    /// Takes `router` and `network` as `configure_routed_path_max_npdu`
    /// does, and waits the same way. Sends nothing.
    fn clear_routed_path_limit<'py>(
        &self,
        py: Python<'py>,
        router: Bound<'py, PyAny>,
        network: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let router = RouterArg::read(&router)?;
        let network = network_arg(&network)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let mac = router.on(&client).await?;
            client
                .clear_routed_path_limit(&mac, network)
                .await
                .map_err(path_err)
        })
    }

    /// Every routed path that holds more than the default, as a list of
    /// `RoutedPathLimit`, by router and then network: a limit set by hand,
    /// one learned from a router's too-long reject, or a dropped
    /// below-minimum reject (`below_minimum_reject_seen`). Sends nothing.
    /// Raises RuntimeError before start and after stop.
    fn routed_path_limits<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let style = AddressStyle::of(client.transport());
            Ok(client
                .routed_path_limits()
                .into_iter()
                .map(|path| PyRoutedPathLimit::new(path, style))
                .collect::<Vec<_>>())
        })
    }

    /// How many too-long rejects (Reject-Message-To-Network, reason 4) the
    /// client dropped since start because they could answer a larger frame
    /// of an earlier request on the same path. These clear by themselves.
    /// Raises RuntimeError before start and after stop.
    fn too_long_reject_drops<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            Ok(started(&inner).await?.too_long_reject_drops())
        })
    }

    /// How many too-long rejects the client dropped since start because the
    /// request they landed on sent nothing above 228 NPDU octets. A rise can
    /// mean a router that carries less than that on some path: its requests
    /// end by timeout until `configure_routed_path_max_npdu` sets the
    /// path's limit. `routed_path_limits()` marks the paths. Raises
    /// RuntimeError before start and after stop.
    fn below_minimum_reject_drops<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            Ok(started(&inner).await?.below_minimum_reject_drops())
        })
    }
}
