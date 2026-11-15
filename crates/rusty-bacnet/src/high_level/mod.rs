//! The high-level calls (#2561): `bacnet-ops`' client over the binding's
//! own low-level client, so a Python call checks what it would send as the
//! library's high-level calls do, and its values come back in both forms
//! ([`PyValue`](value::PyValue): typed, and as text).
//!
//! Each started `BACnetClient` builds one [`OpsClient`] over its low-level
//! client ([`client`]): the two share the client, its discovery table, and
//! one set of request limits the 223 export uses too. A call makes a
//! handle for its device from that client, then runs one of the functions
//! here on the handle. They take the handle, not an address, so another
//! way of naming a device (a target in the string grammar, or a Python
//! `Device` handle) runs the same code.

mod errors;
mod priorities;
mod value;

use std::sync::Arc;

use bacnet_client::client::BACnetClient;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::{DeviceAddress, Priority, WritePolicy};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::app_transport::AppTransport;

pub(crate) use errors::ops_err;
pub(crate) use priorities::{PyPriorityView, PyWritePolicy, priority_arg};

/// The high-level client over the binding's transport.
pub(crate) type OpsClient = bacnet_ops::Client<AppTransport>;

/// A device handle of an [`OpsClient`].
pub(crate) type OpsDevice = bacnet_ops::Device<AppTransport>;

/// The high-level client over `core`, whose writes go under `policy`: no
/// rate cap beyond the client's own `min_request_interval_ms`, and one
/// request at a time to each device.
pub(crate) fn client(core: Arc<BACnetClient<AppTransport>>, policy: WritePolicy) -> OpsClient {
    OpsClient::new(core, Arc::new(RequestLimits::new(0))).with_write_policy(policy)
}

/// The handle for the station at `mac` on this client's own network, an
/// address as `read_property` takes one. It is pinned there and made at
/// once: nothing is sent, and the discovery table needn't hold the device.
/// A MAC the client's link can't send to raises ValueError.
pub(crate) async fn station(client: &OpsClient, mac: &[u8]) -> PyResult<OpsDevice> {
    let address = DeviceAddress::local(mac).map_err(PyValueError::new_err)?;
    client.device(address).await.map_err(ops_err)
}

/// [`Device::relinquish`](bacnet_ops::Device::relinquish) of `object` at
/// `priority`, as a Python call returns it.
pub(crate) async fn relinquish(
    device: &OpsDevice,
    object: ObjectIdentifier,
    priority: Priority,
) -> PyResult<PyPriorityView> {
    let view = device.relinquish(object, priority).await.map_err(ops_err)?;
    crate::py_async::attach(|py| PyPriorityView::new(py, &view))
}

/// [`Device::priority_array`](bacnet_ops::Device::priority_array) of
/// `object`, as a Python call returns it.
pub(crate) async fn priority_array(
    device: &OpsDevice,
    object: ObjectIdentifier,
) -> PyResult<PyPriorityView> {
    let view = device.priority_array(object).await.map_err(ops_err)?;
    crate::py_async::attach(|py| PyPriorityView::new(py, &view))
}

/// Add the classes and exceptions to the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<value::PyValue>()?;
    m.add_class::<PyPriorityView>()?;
    m.add_class::<priorities::PyWritten>()?;
    m.add_class::<PyWritePolicy>()?;
    errors::register(m)
}

#[cfg(test)]
mod tests;
