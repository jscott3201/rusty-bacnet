//! Immutable Python configuration for the existing SC reconnect policy.

use bacnet_transport::sc::ScReconnectConfig;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Reconnect settings for a Python SC client.
///
/// Delay and retry settings use the native transport policy.
#[pyclass(
    name = "ScReconnect",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyScReconnect {
    /// Validated Rust policy copied into each configured client.
    pub(crate) config: ScReconnectConfig,
}

#[pymethods]
impl PyScReconnect {
    /// Create validated reconnect settings; all arguments are keyword-only.
    #[new]
    #[pyo3(signature = (*, initial_delay_ms=10_000, max_delay_ms=600_000, max_retries=10, retry_forever=false))]
    fn new(
        initial_delay_ms: u64,
        max_delay_ms: u64,
        max_retries: u32,
        retry_forever: bool,
    ) -> PyResult<Self> {
        let config = ScReconnectConfig {
            initial_delay_ms,
            max_delay_ms,
            max_retries,
            retry_forever,
        };
        config
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { config })
    }

    /// The minimum reconnect sleep, in milliseconds.
    #[getter]
    fn initial_delay_ms(&self) -> u64 {
        self.config.initial_delay_ms
    }

    /// The maximum reconnect sleep, in milliseconds.
    #[getter]
    fn max_delay_ms(&self) -> u64 {
        self.config.max_delay_ms
    }

    /// The number of reconnect rounds when unlimited recovery is off.
    #[getter]
    fn max_retries(&self) -> u32 {
        self.config.max_retries
    }

    /// Whether to retry eligible hubs without a round limit.
    #[getter]
    fn retry_forever(&self) -> bool {
        self.config.retry_forever
    }

    fn __repr__(&self) -> String {
        format!(
            "ScReconnect(initial_delay_ms={}, max_delay_ms={}, max_retries={}, retry_forever={})",
            self.config.initial_delay_ms,
            self.config.max_delay_ms,
            self.config.max_retries,
            if self.config.retry_forever {
                "True"
            } else {
                "False"
            }
        )
    }
}

/// The `sc_failover_hub` and `sc_connect_timeout_ms` keywords (#1829), on
/// `BACnetClient` and `ScEndpoint` (#1946): SC only, and a failover hub only
/// with `reconnect`, since without reconnection the owner would never turn to
/// it or come back. Each is refused with `ValueError` at construction, before
/// any I/O.
pub(crate) fn hub_options(
    transport: &str,
    reconnect: bool,
    failover_hub: Option<&str>,
    connect_timeout_ms: Option<u64>,
) -> PyResult<()> {
    if transport != "sc" && (failover_hub.is_some() || connect_timeout_ms.is_some()) {
        return Err(PyValueError::new_err(
            "sc_failover_hub and sc_connect_timeout_ms apply only to transport='sc'",
        ));
    }
    if failover_hub.is_some() && !reconnect {
        return Err(PyValueError::new_err(
            "sc_failover_hub requires reconnect=ScReconnect(...)",
        ));
    }
    if connect_timeout_ms == Some(0) {
        return Err(PyValueError::new_err(
            "sc_connect_timeout_ms must be at least 1",
        ));
    }
    Ok(())
}
