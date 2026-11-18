//! The read-only routing-table query (#2612): `routing_table(router)`.
//!
//! One Initialize-Routing-Table with a port count of zero goes to a router
//! on the client's own link, and the acknowledgements are collected into a
//! frozen `RoutingTable`. The binding has no way to send the update form:
//! the Rust call takes no payload.

use pyo3::exceptions::PyTypeError;
use pyo3::types::PyString;

use super::super::*;
use crate::discovery::PyRoutingTable;
use crate::errors::routing_table_err;

/// A bound of the call, in seconds from 0 to 3600 (an hour), as the
/// discovery calls' `wait` is. NaN and the rest raise `ValueError`.
fn seconds(value: f64, name: &str) -> PyResult<std::time::Duration> {
    if !(0.0..=3600.0).contains(&value) {
        return Err(PyValueError::new_err(format!(
            "{name} is a number of seconds from 0 to 3600, not {value}"
        )));
    }
    Ok(std::time::Duration::from_secs_f64(value))
}

/// The router as the caller wrote it, before the started client's link can
/// read a string. The routed-path calls take it too (#2667).
pub(super) enum RouterArg {
    Mac(Vec<u8>),
    Text(String),
}

impl RouterArg {
    pub(super) fn read(router: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(text) = router.cast::<PyString>() {
            return Ok(Self::Text(text.to_cow()?.into_owned()));
        }
        if let Ok(octets) = router.cast::<PyBytes>() {
            return Ok(Self::Mac(octets.as_bytes().to_vec()));
        }
        Err(PyTypeError::new_err(format!(
            "router must be str or bytes, not {}",
            router.get_type().name()?
        )))
    }

    /// The router's MAC on `client`'s link: bytes as given, or a string in
    /// the target grammar's address forms (a B/IPv6 socket address asks the
    /// node its VMAC). `refusal` is the ValueError's text for a string that
    /// names a device or a routed station instead of an address.
    pub(super) async fn on(self, client: &CoreClient, refusal: &str) -> PyResult<Vec<u8>> {
        match self {
            Self::Mac(mac) => Ok(mac),
            Self::Text(text) => bacnet_ops::target::address_mac(client.transport(), &text, refusal)
                .await
                .map_err(|reason| PyValueError::new_err(crate::high_level::link_hint(reason))),
        }
    }
}

#[pymethods]
impl BACnetClient {
    /// Read the routing table of the router at `router`, a station on this
    /// client's own link, without changing it, and return a frozen
    /// `RoutingTable`: its `entries`, how many acknowledgements carried
    /// them (`acks`), and whether a bound cut the answer short
    /// (`truncated`).
    ///
    /// `router` is the router's address on the local link: `bytes` as
    /// given, such as a `DiscoveredRouter`'s `mac`, or a string in the
    /// binding's address forms (`"ip:port"`, `"bip6:N"`, hex octets, an
    /// MS/TP station). An empty or broadcast address raises `ValueError`.
    ///
    /// A long table comes as several acknowledgements with nothing to mark
    /// the last, so the call returns once `quiet` seconds pass after the
    /// last acknowledgement from the router (other traffic does not restart
    /// it), or at `total` seconds from the send, or at `max_entries` (4096
    /// by default, at least 1); the last two set `truncated`, which covers
    /// those bounds only. A quiet end does not prove the table whole: on
    /// MS/TP, acknowledgements after the first wait for the router's token,
    /// so use a larger `quiet` on a slow trunk.
    ///
    /// A router with an empty table returns a `RoutingTable` with no
    /// entries, as does an older router's acknowledgement with no data. A
    /// router that sends nothing within `wait` seconds raises
    /// `BacnetTimeoutError`: one that ignores the message looks the same as
    /// one that is absent (on B/IP, so does one that answers from another
    /// address or port), and neither is an empty table. A router that
    /// refuses with a Reject-Message-To-Network before any acknowledgement
    /// raises `BacnetNetworkRejectError` with its `reason` and `network`;
    /// rejects with reasons 1, 2 and 4 concern a DNET and are ignored. The
    /// bounds are 0 to 3600 seconds, and `wait` may not exceed `total`
    /// (`ValueError` otherwise). A client stopped mid-call raises
    /// `RuntimeError`. A router behind another router can't be asked: the
    /// query is a local message. One query runs at a time per client; a
    /// second call waits for the first. The call holds the started client
    /// for the whole wait, so `stop()` waits for it.
    #[pyo3(signature = (router, *, wait=3.0, quiet=0.5, total=10.0, max_entries=4096))]
    fn routing_table<'py>(
        &self,
        py: Python<'py>,
        router: Bound<'py, PyAny>,
        wait: f64,
        quiet: f64,
        total: f64,
        max_entries: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let router = RouterArg::read(&router)?;
        let bounds = client::RoutingTableWait {
            first_answer: seconds(wait, "wait")?,
            quiet: seconds(quiet, "quiet")?,
            total: seconds(total, "total")?,
            max_entries,
        };
        if max_entries == 0 {
            return Err(PyValueError::new_err("max_entries must be at least 1"));
        }
        if bounds.first_answer > bounds.total {
            return Err(PyValueError::new_err(format!(
                "wait ({wait}) must not be longer than total ({total})"
            )));
        }
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let c = started(&inner).await?;
            let mac = router
                .on(&c, "a routing-table query goes to a router's address")
                .await?;
            let table = c
                .routing_table(&mac, bounds)
                .await
                .map_err(routing_table_err)?;
            Ok(PyRoutingTable(table))
        })
    }
}
