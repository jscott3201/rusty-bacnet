//! The high-level calls on the client itself (#2561): `device`, which
//! gives the `Device` handle every other one hangs off (see
//! [`crate::high_level`]), and the two settings its handles go by.

use super::super::*;
use crate::high_level::{self, PyDevice, PyWritePolicy};

#[pymethods]
impl BACnetClient {
    /// A handle on one device, a `Device`, whose methods are the
    /// high-level calls: `read`, `read_many`, `read_object`, `write`,
    /// `preview_write`, `relinquish`, `priority_array`, `objects` and
    /// `summary`.
    ///
    /// `target` names the device in the target grammar, not in the
    /// `address` forms the typed calls take. Most addresses read the same
    /// in both: `"ip:port"`, six hex octets, the `bip6:` forms and
    /// `"mstp:N"`. Two don't. **A bare `"5"` is device instance 5 here,
    /// not MS/TP station 5: write `"mstp:5"`.** And colon-separated hex
    /// of another length than six octets, which `address` takes as a MAC,
    /// is no address here: it is refused, or, for two decimal parts such
    /// as `"10:20"`, read as instance 20 on network 10. The forms:
    ///
    /// - an address pins the handle there and sends nothing for a station
    ///   on this network: `"192.168.1.10"` or `"192.168.1.10:47808"`,
    ///   `"mstp:5"`, `"bip6:0x0004D2"`, an SC VMAC as six hex octets, a
    ///   routed station as `"2:0x05"` (network 2, MAC 05), or a
    ///   `(network, mac)` pair such as a lookup error's candidate;
    /// - an instance, `1234` (an int or text) or `"2:1234"` on network 2,
    ///   gives a handle that follows the instance: the one address
    ///   claiming it, or the one that answers a Who-Is for it, sent to
    ///   this network alone for a bare instance and to the network named
    ///   otherwise, waiting up to 3 seconds;
    /// - a `discover()` row or a `DiscoveredDevice` pins the handle to its
    ///   address, knowing its instance.
    ///
    /// A wrong type raises TypeError and text the grammar refuses
    /// ValueError, before the Future is made. On await: an address the
    /// client's link can't reach raises ValueError; an instance no address
    /// claims and none answered for, or a routed address with no router
    /// known, BacnetLookupError; an instance two or more addresses claim
    /// BacnetAmbiguousDeviceError, whose `candidates` are each a target.
    #[pyo3(signature = (target))]
    fn device<'py>(
        &self,
        py: Python<'py>,
        target: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let named = high_level::target_arg(target)?;
        let tie = DeviceTie::new(self.inner.clone());
        crate::py_async::future_into_py(py, PyDevice::open(tie, named))
    }

    /// Where a `device()` handle made from a bare instance asks for it, as
    /// `resolve_scope=` set it: `"local"`, `"global"` or a network number;
    /// None on a router's application given none, which asks nowhere.
    #[getter]
    fn resolve_scope(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        use bacnet_ops::ResolveScope;
        Ok(match (self.resolve_scope, self.transport_type.as_str()) {
            (Some(ResolveScope::Network(network)), _) => {
                network.into_pyobject(py)?.into_any().unbind()
            }
            (Some(ResolveScope::Global), _) => "global".into_pyobject(py)?.into_any().unbind(),
            (None, "router") => py.None(),
            _ => "local".into_pyobject(py)?.into_any().unbind(),
        })
    }

    /// The priorities `write` and `relinquish` may use, as `write_policy=`
    /// set them.
    #[getter]
    fn write_policy(&self) -> PyWritePolicy {
        PyWritePolicy(self.write_policy)
    }
}
