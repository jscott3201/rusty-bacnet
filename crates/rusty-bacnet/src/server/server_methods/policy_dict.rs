//! Reading the constructor's policy dicts (`cov_policy`, `time_sync_policy`,
//! and the endpoints' `cov`):
//! each key names a field, and the errors keep the exception types the
//! constructor's other keywords raise.

use pyo3::exceptions::{PyOverflowError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// The dict's entries in order, each key read as a `str`; a non-str key
/// raises TypeError. `None` has no entries.
pub(crate) fn entries<'py>(
    policy: &str,
    dict: Option<&Bound<'py, PyDict>>,
) -> PyResult<Vec<(String, Bound<'py, PyAny>)>> {
    dict.into_iter()
        .flat_map(|dict| dict.iter())
        .map(|(key, value)| {
            let key: String = key
                .extract()
                .map_err(|_| PyTypeError::new_err(format!("{policy} keys must be str")))?;
            Ok((key, value))
        })
        .collect()
}

/// The TypeError for a key the policy does not have.
pub(crate) fn unexpected_key(policy: &str, key: &str) -> PyErr {
    PyTypeError::new_err(format!("{policy} got an unexpected key '{key}'"))
}

/// Extract one value, naming its key in a TypeError or OverflowError while
/// keeping the exception type.
pub(crate) fn field<'py, T: FromPyObjectOwned<'py>>(
    policy: &str,
    key: &str,
    value: &Bound<'py, PyAny>,
) -> PyResult<T> {
    value.extract::<T>().map_err(|error| {
        let py = value.py();
        let error: PyErr = error.into();
        let message = format!("{policy}['{key}']: {}", error.value(py));
        if error.is_instance_of::<PyOverflowError>(py) {
            PyOverflowError::new_err(message)
        } else if error.is_instance_of::<PyTypeError>(py) {
            PyTypeError::new_err(message)
        } else {
            error
        }
    })
}
