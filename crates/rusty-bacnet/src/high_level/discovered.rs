//! [`PyDiscovered`]: one device that answered a `discover()` (#2561).

use bacnet_ops::device::Discovered;
use bacnet_ops::target::AddressStyle;
use pyo3::prelude::*;

use crate::types::PyDiscoveredDevice;

/// One device that answered a `BACnetClient.discover()`: where it answers,
/// its instance, and whether another address claims the same instance. A
/// `BACnetClient.device()` target, which pins the handle to `address`.
/// Refuses `copy` and `pickle`: `entry` counts from this process's clock.
#[pyclass(name = "Discovered", module = "rusty_bacnet", frozen)]
pub(crate) struct PyDiscovered {
    found: Discovered,
    address: String,
}

impl PyDiscovered {
    /// `found`, its address as a link of `style` writes it.
    pub(crate) fn new(found: Discovered, style: AddressStyle) -> Self {
        Self {
            address: found.address.styled(style),
            found,
        }
    }

    /// The row, as `device()` takes it.
    pub(crate) fn found(&self) -> &Discovered {
        &self.found
    }
}

#[pymethods]
impl PyDiscovered {
    /// Where the device answers, as text `device()` reads back:
    /// `"192.168.1.10:47808"`, `"mstp:5"`, `"bip6:0x0004D2"`, an SC VMAC
    /// in hex, or a routed station as `"2:0x05"`.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The network it is on: 0 for this client's own, else the number of
    /// the remote network a router passed its answer on from.
    #[getter]
    fn network(&self) -> u16 {
        self.found.address.network()
    }

    /// Its Device instance.
    #[getter]
    fn instance(&self) -> u32 {
        self.found.instance
    }

    /// True when another address claims the same instance: name this one
    /// by its address, or by this row, never by the instance.
    #[getter]
    fn duplicate(&self) -> bool {
        self.found.duplicate
    }

    /// Its row in the discovery table, a `DiscoveredDevice`: the largest
    /// APDU it takes, its segmentation support and its vendor.
    #[getter]
    fn entry(&self) -> PyDiscoveredDevice {
        PyDiscoveredDevice::from_rust(self.found.entry.clone())
    }

    fn __repr__(&self) -> String {
        let duplicate = if self.found.duplicate {
            ", duplicate"
        } else {
            ""
        };
        format!(
            "Discovered({}, instance={}{duplicate})",
            self.address, self.found.instance
        )
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
