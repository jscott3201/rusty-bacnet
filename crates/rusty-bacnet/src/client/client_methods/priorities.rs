//! The priority calls (#2561): `relinquish` and `priority_array`, through
//! the high-level client (see [`crate::high_level`]).

use super::super::*;
use crate::high_level::{self, PyWritePolicy};

#[pymethods]
impl BACnetClient {
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

    /// The priorities `relinquish` may use, as `write_policy=` set them.
    #[getter]
    fn write_policy(&self) -> PyWritePolicy {
        PyWritePolicy(self.write_policy)
    }
}
