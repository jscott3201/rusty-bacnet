//! `RoutedPathLimit` (#2667): one routed path's size limit, as
//! `BACnetClient.routed_path_limits()` lists it.

use bacnet_client::client::RoutedPathLimit;
use bacnet_ops::target::AddressStyle;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// One routed path, the router this client sends through and the
/// destination network, with the size limit the client holds for it.
#[pyclass(name = "RoutedPathLimit", module = "rusty_bacnet", frozen)]
pub(crate) struct PyRoutedPathLimit {
    path: RoutedPathLimit,
    /// The router's MAC in its link's own form, for `repr`.
    router_text: String,
}

impl PyRoutedPathLimit {
    /// `path`, with its router written as `style` writes the link's MACs.
    pub(crate) fn new(path: RoutedPathLimit, style: AddressStyle) -> Self {
        let router_text = style.format(&path.router_mac);
        Self { path, router_text }
    }
}

#[pymethods]
impl PyRoutedPathLimit {
    /// The router's MAC on the local link, as `DiscoveredRouter.mac` gives
    /// it; the routed-path calls take it back.
    #[getter]
    fn router<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.path.router_mac)
    }

    /// The destination network.
    #[getter]
    fn network(&self) -> u16 {
        self.path.dnet
    }

    /// The limit requests on the path are sized to, in NPDU octets.
    #[getter]
    fn max_npdu(&self) -> u16 {
        self.path.max_npdu()
    }

    /// Where `max_npdu` comes from: `"default"` (228 octets),
    /// `"configured"`, or `"learned"` from a router's too-long reject.
    #[getter]
    fn source(&self) -> &'static str {
        self.path.source().name()
    }

    /// The limit set with `configure_routed_path_max_npdu`, or None.
    #[getter]
    fn configured_max_npdu(&self) -> Option<u16> {
        self.path.configured_max_npdu
    }

    /// The limit learned from the path's too-long rejects, or None.
    #[getter]
    fn learned_max_npdu(&self) -> Option<u16> {
        self.path.learned_max_npdu
    }

    /// Whether a too-long reject for a request of 228 NPDU octets or fewer
    /// was dropped on this path since its limit was last set or cleared.
    #[getter]
    fn below_minimum_reject_seen(&self) -> bool {
        self.path.below_minimum_reject_seen
    }

    fn __repr__(&self) -> String {
        format!(
            "RoutedPathLimit(router='{}', network={}, max_npdu={}, source='{}')",
            self.router_text,
            self.path.dnet,
            self.path.max_npdu(),
            self.path.source().name()
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRoutedPathLimit>()?;
    Ok(())
}
