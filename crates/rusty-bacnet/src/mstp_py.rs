//! Helpers for constructing MS/TP transports from Python kwargs.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::PyResult;

use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::{MstpConfig, MstpTransport};
use bacnet_transport::mstp_serial::{SerialConfig, TokioSerialPort};
use bacnet_types::error::Error;

/// Serial port type used by the Python bindings' [`AnyTransport`] parameter.
pub type PySerial = TokioSerialPort;

const SUPPORTED_MSTP_BAUD_RATES: [u32; 6] = [9_600, 19_200, 38_400, 57_600, 76_800, 115_200];
const SUPPORTED_MSTP_BAUD_ERROR: &str =
    "mstp_baud must be one of 9600, 19200, 38400, 57600, 76800, or 115200";

fn is_supported_mstp_baud(rate: u32) -> bool {
    SUPPORTED_MSTP_BAUD_RATES.contains(&rate)
}

/// Validate Python-owned MS/TP configuration without opening the serial device.
pub(crate) fn validate_mstp_config(
    serial_port: Option<&str>,
    mstp_baud: u32,
    mstp_mac: u8,
    mstp_max_master: u8,
    mstp_max_info_frames: u8,
) -> PyResult<&str> {
    let path = serial_port
        .ok_or_else(|| PyValueError::new_err("serial_port is required for transport='mstp'"))?;
    if !is_supported_mstp_baud(mstp_baud) {
        return Err(PyValueError::new_err(SUPPORTED_MSTP_BAUD_ERROR));
    }
    if mstp_mac > 127 {
        return Err(PyValueError::new_err("mstp_mac must be in 0..=127"));
    }
    if mstp_max_master > 127 {
        return Err(PyValueError::new_err("mstp_max_master must be in 0..=127"));
    }
    if mstp_mac > mstp_max_master {
        return Err(PyValueError::new_err("mstp_mac must be <= mstp_max_master"));
    }
    if mstp_max_info_frames == 0 {
        return Err(PyValueError::new_err(
            "mstp_max_info_frames must be in 1..=255",
        ));
    }
    Ok(path)
}

/// Open an MS/TP transport from Python kwargs.
pub fn build_mstp_transport(
    serial_port: Option<&str>,
    mstp_baud: u32,
    mstp_mac: u8,
    mstp_max_master: u8,
    mstp_max_info_frames: u8,
) -> PyResult<AnyTransport<PySerial>> {
    let path = validate_mstp_config(
        serial_port,
        mstp_baud,
        mstp_mac,
        mstp_max_master,
        mstp_max_info_frames,
    )?;
    let serial = TokioSerialPort::open(&SerialConfig {
        port_name: path.to_string(),
        baud_rate: mstp_baud,
    })
    .map_err(|error| {
        let message = match error {
            Error::Encoding(message) => message,
            other => other.to_string(),
        };
        PyRuntimeError::new_err(message)
    })?;
    let config = MstpConfig {
        this_station: mstp_mac,
        max_master: mstp_max_master,
        max_info_frames: mstp_max_info_frames,
        baud_rate: mstp_baud,
    };
    Ok(AnyTransport::Mstp(MstpTransport::new(serial, config)))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pyo3::exceptions::{PyRuntimeError, PyValueError};
    use pyo3::{PyErr, Python};

    use super::*;

    fn assert_py_error<T>(result: PyResult<T>, message: &str, is_value_error: bool) {
        let err = match result {
            Ok(_) => panic!("expected Python error"),
            Err(err) => err,
        };
        Python::attach(|py| {
            if is_value_error {
                assert!(err.is_instance_of::<PyValueError>(py));
            } else {
                assert!(err.is_instance_of::<PyRuntimeError>(py));
            }
            assert_eq!(err.value(py).to_string(), message);
        });
    }

    fn validate(
        baud: u32,
        mac: u8,
        max_master: u8,
        max_info_frames: u8,
    ) -> Result<&'static str, PyErr> {
        validate_mstp_config(
            Some("intentionally-nonexistent-serial-device"),
            baud,
            mac,
            max_master,
            max_info_frames,
        )
    }

    #[test]
    fn rejects_invalid_configuration_before_open() {
        assert_py_error(
            validate_mstp_config(None, 38_400, 1, 127, 1),
            "serial_port is required for transport='mstp'",
            true,
        );
        for baud in [0, 12_345] {
            assert_py_error(validate(baud, 1, 127, 1), SUPPORTED_MSTP_BAUD_ERROR, true);
        }
        assert_py_error(
            validate(38_400, 128, 127, 1),
            "mstp_mac must be in 0..=127",
            true,
        );
        assert_py_error(
            validate(38_400, 1, 128, 1),
            "mstp_max_master must be in 0..=127",
            true,
        );
        assert_py_error(
            validate(38_400, 4, 3, 1),
            "mstp_mac must be <= mstp_max_master",
            true,
        );
        assert_py_error(
            validate(38_400, 1, 127, 0),
            "mstp_max_info_frames must be in 1..=255",
            true,
        );
    }

    #[test]
    fn accepts_all_supported_baud_rates() {
        for baud in SUPPORTED_MSTP_BAUD_RATES {
            assert!(validate(baud, 1, 127, 1).is_ok(), "baud {baud}");
        }
    }

    #[test]
    fn valid_configuration_reaches_serial_open() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir()
            .join(format!("rusty-bacnet-missing-{unique}"))
            .join("serial-device");
        let result =
            build_mstp_transport(Some(path.to_string_lossy().as_ref()), 115_200, 1, 127, 255);
        let err = match result {
            Ok(_) => panic!("nonexistent serial device unexpectedly opened"),
            Err(err) => err,
        };
        Python::attach(|py| {
            assert!(err.is_instance_of::<PyRuntimeError>(py));
            let message = err.value(py).to_string();
            assert!(message.starts_with("Serial open failed"), "{message}");
            assert_eq!(
                message.matches("Serial open failed").count(),
                1,
                "{message}"
            );
        });
    }
}
