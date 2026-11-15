//! The high-level calls on the client itself (#2561): `device`, which
//! gives the `Device` handle the rest hang off, and `relinquish` and
//! `priority_array` by address, through the high-level client (see
//! [`crate::high_level`]).

use super::super::*;
use crate::high_level::{self, PyDevice, PyWritePolicy};

#[pymethods]
impl BACnetClient {
    /// A handle on one device, a `Device`, whose methods are the
    /// high-level calls: `read`, `read_many`, `read_object`, `write`,
    /// `preview_write`, `relinquish`, `priority_array`, `objects` and
    /// `summary`.
    ///
    /// `target` names the device in the target grammar, never in the
    /// `address` forms the typed calls take (there a bare `"5"` is an
    /// MS/TP station; here it is device instance 5):
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

    /// Release a command: NULL written to the object's Present_Value at
    /// `priority`, so the next slot down, or Relinquish_Default, takes
    /// over. Then the object's priorities as read just after: one
    /// WriteProperty, then one ReadPropertyMultiple, or a ReadProperty of
    /// each of the three properties when the device refuses that.
    ///
    /// `address` is a station on this client's own network, in the forms
    /// `read_property` takes: always an address, never a device instance.
    /// `priority` is required and is an int from 1 to 16: a bool, a float
    /// or anything else raises TypeError, and another int ValueError, as
    /// does address text the parser refuses, before the Future is made.
    /// On await, with nothing sent: an address the client's link can't
    /// carry raises ValueError, as does an object type with no
    /// Priority_Array, and a priority the client's `write_policy` refuses
    /// (1, 2, 5 and 6 unless allowed) BacnetWriteDeniedError. Then the
    /// device's refusal of the write raises the usual protocol, reject or
    /// abort error, and no answer BacnetTimeoutError (where
    /// `write_property` raises BacnetAbortError, reason 10). A relinquish
    /// the device acknowledged whose read after it failed raises
    /// BacnetReadBackError, and one whose read says the object has no
    /// Priority_Array raises BacnetNoPriorityArrayError; both say what was
    /// written (`written`), which needs no second send.
    #[pyo3(signature = (address, object_id, priority))]
    fn relinquish<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        priority: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = high_level::priority_arg(&priority)?;
        let mac = PeerAddresses::for_transport(&self.transport_type).parse(&address)?;
        let object = object_id.to_rust();
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let high = state::high_level(&inner).await?;
            let device = high_level::station(&high, &mac).await?;
            high_level::relinquish(&device, object, priority).await
        })
    }

    /// Read an object's commands: its Priority_Array, Relinquish_Default
    /// and Present_Value in one ReadPropertyMultiple (a ReadProperty of
    /// each when the device refuses that), as a PriorityView.
    ///
    /// `address` is as `relinquish` takes it. Address text the parser
    /// refuses raises ValueError before the Future is made; an address the
    /// link can't carry, or an object type with no Priority_Array, raises
    /// ValueError on await with nothing sent. A device that says the object
    /// has none raises BacnetNoPriorityArrayError, and no answer
    /// BacnetTimeoutError.
    #[pyo3(signature = (address, object_id))]
    fn priority_array<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mac = PeerAddresses::for_transport(&self.transport_type).parse(&address)?;
        let object = object_id.to_rust();
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let high = state::high_level(&inner).await?;
            let device = high_level::station(&high, &mac).await?;
            high_level::priority_array(&device, object).await
        })
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
