//! Immutable Python configuration for the existing SC reconnect policy.

use bacnet_transport::sc::ScReconnectConfig;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Reconnect settings for a Python SC client.
///
/// Delay validation uses the Rust policy. Zero retries permits the initial
/// connection and disables retries after a connection loss.
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
    #[pyo3(signature = (*, initial_delay_ms=10_000, max_delay_ms=600_000, max_retries=10))]
    fn new(initial_delay_ms: u64, max_delay_ms: u64, max_retries: u32) -> PyResult<Self> {
        let config = ScReconnectConfig {
            initial_delay_ms,
            max_delay_ms,
            max_retries,
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

    /// The maximum retry attempts after an active connection is lost.
    #[getter]
    fn max_retries(&self) -> u32 {
        self.config.max_retries
    }

    fn __repr__(&self) -> String {
        format!(
            "ScReconnect(initial_delay_ms={}, max_delay_ms={}, max_retries={})",
            self.config.initial_delay_ms, self.config.max_delay_ms, self.config.max_retries
        )
    }
}
