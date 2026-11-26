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

/// The ValueError for a router string that names a device or a routed
/// station rather than the router's address on the local link.
const NOT_A_ROUTER: &str = "a routed path names its router by its address on this client's \
                            link, not a device instance or a station behind a router";

/// An `int` argument as the caller passed it, and not a `bool`, as
/// `RoutedTarget` takes its network: OverflowError outside 0 to 65535,
/// TypeError for another type. The client checks the range.
fn int_arg(value: &Bound<'_, PyAny>, name: &str) -> PyResult<u16> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} must be an int, not bool"
        )));
    }
    value.extract()
}

/// A refused argument (a router MAC of the wrong length, a network outside
/// 1 to 65534, a limit too small to carry a request), the client's
/// [`Error::InvalidArgument`] (#1833), is a ValueError with its text; the
/// rest as every call raises it.
fn path_err(error: Error) -> PyErr {
    match error {
        Error::InvalidArgument(refusal) => PyValueError::new_err(refusal.to_string()),
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
    /// `mac`, or a string in the binding's address forms (a B/IPv6 socket
    /// address is first asked its VMAC). `network` is 1 to 65534. A limit
    /// under 228 octets is taken, for an older router that carries less than
    /// the standard asks; one that could carry no request at all (under 66
    /// on B/IP, 61 on MS/TP) raises ValueError, as do a bad router or
    /// network; an `int` outside 0 to 65535 raises OverflowError, a `bool`
    /// TypeError. The change applies at once (#2683): it waits neither for
    /// a request on the path nor out the hold a request with no answer
    /// leaves; a request waiting out the hold goes sized to the new limit
    /// when it ends. Nothing goes through the path. Raises RuntimeError
    /// before start and after stop.
    fn configure_routed_path_max_npdu<'py>(
        &self,
        py: Python<'py>,
        router: Bound<'py, PyAny>,
        network: Bound<'py, PyAny>,
        max_npdu: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let router = RouterArg::read(&router)?;
        let network = int_arg(&network, "network")?;
        let max_npdu = int_arg(&max_npdu, "max_npdu")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let mac = router.on(&client, NOT_A_ROUTER).await?;
            client
                .configure_routed_path_max_npdu(&mac, network, max_npdu)
                .map_err(path_err)
        })
    }

    /// Put the routed path through `router` to `network` back on the
    /// default, 228 octets, dropping the limit set or learned for it.
    /// Takes `router` and `network` as `configure_routed_path_max_npdu`
    /// does, and applies at once too. Nothing goes through the path.
    fn clear_routed_path_limit<'py>(
        &self,
        py: Python<'py>,
        router: Bound<'py, PyAny>,
        network: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let router = RouterArg::read(&router)?;
        let network = int_arg(&network, "network")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let mac = router.on(&client, NOT_A_ROUTER).await?;
            client
                .clear_routed_path_limit(&mac, network)
                .map_err(path_err)
        })
    }

    /// Every routed path that holds more than the default, as a list of
    /// `RoutedPathLimit`, by router and then network: a limit set by hand,
    /// one learned from a router's too-long reject, or a dropped
    /// below-minimum reject (`below_minimum_reject_seen`). A path held
    /// only by that mark may be dropped when the client's table of 256
    /// paths is full. Sends nothing. Raises RuntimeError before start and
    /// after stop.
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

    /// How many answers for a device's open request the client dropped
    /// since start because they came by another path than the request took
    /// (#2831): from another station than a routed request's router, or
    /// relayed with this network's own number as SNET for a request sent
    /// straight to the device. A rise means a station is answering for
    /// other devices. Raises RuntimeError before start and after stop.
    fn off_path_answer_drops<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            Ok(started(&inner).await?.off_path_answer_drops())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::error::Refusal;

    /// The client refuses a path's router, network or limit as an argument
    /// (#1833), which raises ValueError with its text alone, as the untyped
    /// text it replaces did; a full path table raises as every call does.
    #[test]
    fn a_refused_argument_is_a_value_error() {
        Python::initialize();
        Python::attach(|py| {
            let refused = path_err(Error::invalid_argument("routed DNET must be in 1..=65534"));
            assert!(refused.is_instance_of::<PyValueError>(py));
            assert_eq!(
                refused.value(py).str().unwrap().to_str().unwrap(),
                "routed DNET must be in 1..=65534"
            );
            let short = path_err(Error::InvalidArgument(Refusal::text(
                "a routed path limit of 10 NPDU octets carries no request",
            )));
            assert!(short.is_instance_of::<PyValueError>(py));
            let full = path_err(Error::RoutedPathCapacityExceeded { capacity: 256 });
            assert!(!full.is_instance_of::<PyValueError>(py));
            assert!(full.is_instance_of::<crate::errors::BacnetError>(py));
        });
    }
}
