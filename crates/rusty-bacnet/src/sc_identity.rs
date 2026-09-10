//! Owned node identity at the Python configuration boundary; no I/O or generation.

use pyo3::{exceptions::PyValueError, PyResult};

pub(crate) fn device_uuid(transport: &str, value: Option<Vec<u8>>) -> PyResult<[u8; 16]> {
    // Like the other SC options, this option is unused for non-SC transports.
    if transport != "sc" {
        return Ok([0; 16]);
    }
    let value = value
        .ok_or_else(|| PyValueError::new_err("sc_device_uuid is required for SC transport"))?;
    let uuid: [u8; 16] = value
        .try_into()
        .map_err(|_| PyValueError::new_err("sc_device_uuid must be exactly 16 bytes"))?;
    if uuid == [0; 16] {
        return Err(PyValueError::new_err("sc_device_uuid must not be all zero"));
    }
    Ok(uuid)
}
