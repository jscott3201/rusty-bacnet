//! The high-level calls (#2561): `bacnet-ops`' client over the binding's
//! own low-level client, so a Python call checks what it would send as the
//! library's high-level calls do, and its values come back in both forms
//! ([`PyValue`](value::PyValue): typed, and as text).
//!
//! Each started `BACnetClient` builds one [`OpsClient`] over its low-level
//! client ([`client`]): the two share the client, its discovery table, and
//! one set of request limits the 223 export uses too. Every call goes
//! through a Python `Device` ([`PyDevice`]), made by `BACnetClient.device`
//! from a target in the grammar: its methods are the high-level calls,
//! and there is no second spelling of them on the client.

mod device;
mod discovered;
mod errors;
mod preview;
mod priorities;
mod readings;
mod specs;
mod value;

use std::sync::Arc;

use bacnet_client::client::BACnetClient;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::{Priority, ResolveScope, WritePolicy};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::app_transport::AppTransport;

pub(crate) use device::PyDevice;
pub(crate) use discovered::PyDiscovered;
pub(crate) use errors::ops_err;
pub(crate) use priorities::{PyPriorityView, PyWritePolicy};
pub(crate) use specs::target_arg;

/// The high-level client over the binding's transport.
pub(crate) type OpsClient = bacnet_ops::Client<AppTransport>;

/// A device handle of an [`OpsClient`].
pub(crate) type OpsDevice = bacnet_ops::Device<AppTransport>;

/// The high-level client over `core`, whose writes go under `policy`: no
/// rate cap beyond the client's own `min_request_interval_ms`, and one
/// request at a time to each device.
pub(crate) fn client(
    core: Arc<BACnetClient<AppTransport>>,
    policy: WritePolicy,
    scope: Option<ResolveScope>,
) -> OpsClient {
    OpsClient::new(core, Arc::new(RequestLimits::new(0)))
        .with_write_policy(policy)
        .with_resolve_scope(scope.unwrap_or_default())
}

/// The `resolve_scope=` keyword of `BACnetClient`: where a `device()`
/// handle made from a bare instance (`1234`) asks for it when no address
/// claims it. `"local"`, the default, is this network alone; `"global"`
/// every network; a number from 1 to 65534 that network. A target that
/// names its network (`"5:1234"`) asks there whatever this says. On a
/// router's application (`transport`, `"router"`), which has no network
/// of its own, `"local"` raises ValueError and the default is `None`: a
/// bare instance the client hasn't heard from is then refused, saying how
/// to name it.
pub(crate) fn resolve_scope_arg(
    scope: Option<&Bound<'_, PyAny>>,
    transport: &str,
) -> PyResult<Option<ResolveScope>> {
    use pyo3::exceptions::PyTypeError;
    use pyo3::types::{PyBool, PyInt, PyString};
    const EXPECTED: &str = "resolve_scope is 'local', 'global' or a network number from 1 to 65534";
    let router = transport == "router";
    let Some(scope) = scope.filter(|scope| !scope.is_none()) else {
        return Ok((!router).then_some(ResolveScope::Local));
    };
    if let Ok(name) = scope.cast::<PyString>() {
        return match name.to_str()? {
            "local" if router => Err(PyValueError::new_err(
                "a client on transport='router' has no network of its own for \
                 resolve_scope='local': pass a network number, or 'global'",
            )),
            "local" => Ok(Some(ResolveScope::Local)),
            "global" => Ok(Some(ResolveScope::Global)),
            other => Err(PyValueError::new_err(format!("{EXPECTED}, not '{other}'"))),
        };
    }
    if scope.is_instance_of::<PyInt>() && !scope.is_instance_of::<PyBool>() {
        return match scope.extract::<u16>() {
            Ok(network @ 1..=65534) => Ok(Some(ResolveScope::Network(network))),
            _ => Err(PyValueError::new_err(format!("{EXPECTED}, not {scope}"))),
        };
    }
    Err(PyTypeError::new_err(format!(
        "{EXPECTED}, not {}",
        scope.get_type().name()?
    )))
}

/// A refusal as Python shows it: where the operations core says only
/// which link reaches an address (it names no front end's option), the
/// keyword that picks that link here.
pub(crate) fn link_hint(reason: String) -> String {
    use bacnet_ops::target::{NEEDS_BIP6, NEEDS_MSTP};
    let transport = match () {
        () if reason.ends_with(NEEDS_BIP6) => "ipv6",
        () if reason.ends_with(NEEDS_MSTP) => "mstp",
        () => return reason,
    };
    format!("{reason} (BACnetClient(transport='{transport}'))")
}

/// Why a router's application can't reach the station a local address
/// names (#2071): it has no network of its own. Raised before anything is
/// sent, where the send itself would fail with the router's refusal.
fn no_own_network(shown: &str) -> PyErr {
    PyValueError::new_err(format!(
        "a client on transport='router' has no network of its own, so the local address \
         {shown} names no station it can reach: name the station with its network \
         ('5:0x0A000001BAC0'), or the device by its network and instance ('5:1234')"
    ))
}

/// The handle `named` names on `client`, as `bacnet_ops::Client::device`
/// makes it, with what a router's application can't do refused first:
/// a local address, and a bare instance the discovery table doesn't hold
/// when the client was given no `resolve_scope` to ask in.
pub(crate) async fn device(
    client: &OpsClient,
    named: &specs::Named,
    asks: bool,
) -> PyResult<OpsDevice> {
    use bacnet_ops::device::DeviceRef;
    use bacnet_ops::target::{Target, parse_target};
    let style = client.address_style();
    let router = matches!(client.low_level().transport(), AppTransport::Router(_));
    let bare = match named.as_ref() {
        DeviceRef::Instance(instance) => Some(instance),
        DeviceRef::Text(text) => match parse_target(text) {
            Ok(Target::Instance(instance)) => Some(instance),
            _ => None,
        },
        _ => None,
    };
    if let (true, false, Some(instance)) = (router, asks, bare)
        && matches!(client.low_level().get_device(instance).await, Ok(None))
    {
        return Err(errors::unasked(instance));
    }
    let device = client
        .device(named.as_ref())
        .await
        .map_err(|error| ops_err(error, style))?;
    let address = device.address();
    if router && address.is_local() {
        return Err(no_own_network(&address.styled(style)));
    }
    Ok(device)
}

/// [`Device::relinquish`](bacnet_ops::Device::relinquish) of `object` at
/// `priority`, as a Python call returns it.
pub(crate) async fn relinquish(
    device: &OpsDevice,
    object: ObjectIdentifier,
    priority: Priority,
) -> PyResult<PyPriorityView> {
    let style = device.client().address_style();
    let view = device
        .relinquish(object, priority)
        .await
        .map_err(|e| ops_err(e, style))?;
    crate::py_async::attach(|py| PyPriorityView::new(py, &view))
}

/// [`Device::priority_array`](bacnet_ops::Device::priority_array) of
/// `object`, as a Python call returns it.
pub(crate) async fn priority_array(
    device: &OpsDevice,
    object: ObjectIdentifier,
) -> PyResult<PyPriorityView> {
    let style = device.client().address_style();
    let view = device
        .priority_array(object)
        .await
        .map_err(|e| ops_err(e, style))?;
    crate::py_async::attach(|py| PyPriorityView::new(py, &view))
}

/// Add the classes and exceptions to the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<value::PyValue>()?;
    m.add_class::<PyPriorityView>()?;
    m.add_class::<priorities::PyWritten>()?;
    m.add_class::<PyWritePolicy>()?;
    m.add_class::<PyDevice>()?;
    m.add_class::<PyDiscovered>()?;
    m.add_class::<readings::PyReading>()?;
    m.add_class::<readings::PyReadings>()?;
    m.add_class::<readings::PyObjectList>()?;
    m.add_class::<readings::PyDeviceSummary>()?;
    m.add_class::<preview::PyWritePreview>()?;
    errors::register(m)
}

#[cfg(test)]
mod tests;
