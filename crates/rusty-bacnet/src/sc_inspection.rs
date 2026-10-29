//! Synchronous owned offline reports, with explicit byte and path boundaries.

use crate::sc_peer::PyScCertificateMetadata;
use bacnet_transport::sc_tls::inspection::{
    self, MAX_INPUT_BYTES, ScExtensionInspection, ScInputErrorKind, ScInputRole,
    ScInspectedCertificate, ScInspectionError, ScInspectionFinding, ScKeyFilePermissions,
    ScPemInspection,
};
use pyo3::{
    buffer::PyBuffer,
    exceptions::{PyOSError, PyTypeError, PyValueError},
    prelude::*,
    sync::critical_section::with_critical_section,
    types::{PyBool, PyByteArray, PyBytes, PyInt, PyString, PyTuple},
};
use std::{borrow::Cow, path::PathBuf};

#[pyclass(name = "ScPemInspection", module = "rusty_bacnet", frozen)]
struct PyScPemInspection(ScPemInspection);
#[pymethods]
impl PyScPemInspection {
    #[getter]
    fn inspected_at_unix(&self) -> i64 {
        self.0.inspected_at_unix
    }
    #[getter]
    fn peer_trust_supplied(&self) -> bool {
        self.0.peer_trust_supplied
    }
    #[getter]
    fn key_consistency(&self) -> &'static str {
        self.0.key_consistency.as_str()
    }
    #[getter]
    fn key_file_permissions(&self) -> Option<PyScKeyFilePermissions> {
        self.0.key_file_permissions.map(PyScKeyFilePermissions)
    }
    /// Whether a peer-trust certificate signed the identity leaf directly:
    /// `"direct"`, `"not_direct"`, `"unknown"`, or `"not_assessed"` without
    /// peer trust (#1846).
    #[getter]
    fn direct_issuer(&self) -> &'static str {
        self.0.direct_issuer.as_str()
    }
    /// Zero-based peer-trust index of the CA that signed the leaf, when
    /// `direct_issuer` is `"direct"`; else `None`.
    #[getter]
    fn direct_issuer_index(&self) -> Option<usize> {
        self.0.direct_issuer_index
    }
    #[getter]
    fn certificates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.0
                .certificates
                .iter()
                .map(|v| Py::new(py, PyScInspectedCertificate(v.clone())))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "ScPemInspection(inspected_at_unix={}, certificate_count={}, key_consistency='{}')",
            self.inspected_at_unix(),
            self.0.certificates.len(),
            self.key_consistency()
        )
    }
}

#[pyclass(name = "ScInspectedCertificate", module = "rusty_bacnet", frozen)]
struct PyScInspectedCertificate(ScInspectedCertificate);
#[pymethods]
impl PyScInspectedCertificate {
    #[getter]
    fn role(&self) -> &'static str {
        self.0.role.as_str()
    }
    #[getter]
    fn index(&self) -> usize {
        self.0.index
    }
    #[getter]
    fn sha256<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.sha256)
    }
    #[getter]
    fn metadata(&self) -> Option<PyScCertificateMetadata> {
        self.0.metadata.clone().map(PyScCertificateMetadata)
    }
    #[getter]
    fn validity(&self) -> &'static str {
        self.0.validity.as_str()
    }
    #[getter]
    fn expiry(&self) -> &'static str {
        self.0.expiry.as_str()
    }
    #[getter]
    fn san(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.san.clone())
    }
    #[getter]
    fn key_usage(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.key_usage.clone())
    }
    #[getter]
    fn extended_key_usage(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.extended_key_usage.clone())
    }
    #[getter]
    fn findings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.0
                .findings
                .iter()
                .map(|v| Py::new(py, PyScInspectionFinding(*v)))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "ScInspectedCertificate(role='{}', index={}, validity='{}')",
            self.role(),
            self.index(),
            self.validity()
        )
    }
}

#[pyclass(name = "ScInspectionFinding", module = "rusty_bacnet", frozen)]
struct PyScInspectionFinding(ScInspectionFinding);
#[pymethods]
impl PyScInspectionFinding {
    #[getter]
    fn area(&self) -> &'static str {
        self.0.area.as_str()
    }
    #[getter]
    fn kind(&self) -> &'static str {
        self.0.kind.as_str()
    }
    fn __repr__(&self) -> String {
        format!(
            "ScInspectionFinding(area='{}', kind='{}')",
            self.area(),
            self.kind()
        )
    }
}

#[pyclass(name = "ScExtensionInspection", module = "rusty_bacnet", frozen)]
struct PyScExtensionInspection(ScExtensionInspection);
#[pymethods]
impl PyScExtensionInspection {
    #[getter]
    fn assessed(&self) -> bool {
        self.0.assessed
    }
    #[getter]
    fn present(&self) -> bool {
        self.0.present
    }
    #[getter]
    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.values)
    }
    fn __repr__(&self) -> String {
        format!(
            "ScExtensionInspection(assessed={}, present={}, value_count={})",
            self.assessed(),
            self.present(),
            self.0.values.len()
        )
    }
}

#[pyclass(name = "ScKeyFilePermissions", module = "rusty_bacnet", frozen)]
struct PyScKeyFilePermissions(ScKeyFilePermissions);
#[pymethods]
impl PyScKeyFilePermissions {
    #[getter]
    fn group_readable(&self) -> bool {
        self.0.group_readable
    }
    #[getter]
    fn world_readable(&self) -> bool {
        self.0.world_readable
    }
    fn __repr__(&self) -> String {
        format!(
            "ScKeyFilePermissions(group_readable={}, world_readable={})",
            self.group_readable(),
            self.world_readable()
        )
    }
}

fn time(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<i64>> {
    value
        .map(|value| {
            if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
                return Err(PyTypeError::new_err(
                    "inspected_at_unix must be an integer Unix second",
                ));
            }
            value.extract()
        })
        .transpose()
}
enum PemInput<'a, 'py> {
    Bytes(&'a [u8]),
    Mutable {
        value: &'a Bound<'py, PyByteArray>,
        buffer: PyBuffer<u8>,
    },
}
impl<'a, 'py> PemInput<'a, 'py> {
    fn pin(value: &'a Bound<'py, PyAny>) -> PyResult<Self> {
        match value.cast::<PyBytes>() {
            Ok(value) => Ok(Self::Bytes(value.as_bytes())),
            _ => {
                match value.cast::<PyByteArray>() {
                    Ok(value) => {
                        let buffer = if !value.is_exact_instance_of::<PyByteArray>()
                            && value.py().version_info() >= (3, 12)
                        {
                            // Python 3.12 introduced subclass buffer hooks. Invoke the
                            // built-in descriptor so the export pins the same bytearray
                            // storage we lock below, without calling an override that can
                            // redirect the input to unrelated mutable storage.
                            let view = value
                                .py()
                                .get_type::<PyByteArray>()
                                .getattr("__buffer__")?
                                .call1((value, pyo3::ffi::PyBUF_FULL_RO))?;
                            PyBuffer::get(&view)?
                        } else {
                            PyBuffer::get(value.as_any())?
                        };
                        Ok(Self::Mutable { value, buffer })
                    }
                    _ => Err(PyTypeError::new_err(
                        "PEM content must be bytes or bytearray",
                    )),
                }
            }
        }
    }

    fn snapshot(self, remaining: &mut usize, role: ScInputRole) -> PyResult<Cow<'a, [u8]>> {
        let len = match &self {
            Self::Bytes(value) => value.len(),
            Self::Mutable { buffer, .. } => buffer.len_bytes(),
        };
        if len > *remaining {
            return Err(error(ScInspectionError::Input {
                role,
                index: None,
                kind: ScInputErrorKind::InputTooLarge,
            }));
        }
        let snapshot = match self {
            Self::Bytes(value) => Cow::Borrowed(value),
            Self::Mutable { value, buffer } => {
                // The export pins the checked length through this exact copy,
                // including on free-threaded Python. Lock mutable contents while
                // copying; dropping the export releases it on every exit path.
                Cow::Owned(with_critical_section(value, || buffer.to_vec(value.py()))?)
            }
        };
        *remaining -= snapshot.len();
        Ok(snapshot)
    }
}
fn path(value: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    let value = value
        .py()
        .import("os")?
        .getattr("fspath")?
        .call1((value,))?;
    let value = value
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err("paths must be str or os.PathLike[str]"))?;
    value.extract()
}
fn error(error: ScInspectionError) -> PyErr {
    match error {
        ScInspectionError::Io { source, .. } => match source.raw_os_error() {
            Some(code) => {
                // PyO3's generic io::Error conversion keeps the message but
                // drops errno. Invoke OSError's native constructor explicitly.
                #[cfg(windows)]
                {
                    PyOSError::new_err((0, source.to_string(), None::<String>, code))
                }
                #[cfg(not(windows))]
                {
                    PyOSError::new_err((code, source.to_string()))
                }
            }
            None => PyOSError::new_err(source.to_string()),
        },
        error => PyValueError::new_err(error.to_string()),
    }
}

/// Inspect supplied PEM bytes without constructing a client or contacting a hub.
#[pyfunction]
#[pyo3(signature = (identity_pem, *, peer_trust_pem=None, private_key_pem=None, inspected_at_unix=None))]
fn inspect_sc_pem(
    identity_pem: &Bound<'_, PyAny>,
    peer_trust_pem: Option<&Bound<'_, PyAny>>,
    private_key_pem: Option<&Bound<'_, PyAny>>,
    inspected_at_unix: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyScPemInspection> {
    let now = time(inspected_at_unix)?;
    let mut remaining = MAX_INPUT_BYTES;
    let mut snapshot = |value, role| PemInput::pin(value)?.snapshot(&mut remaining, role);
    let identity = snapshot(identity_pem, ScInputRole::Identity)?;
    let trust = peer_trust_pem
        .map(|value| snapshot(value, ScInputRole::PeerTrust))
        .transpose()?;
    let key = private_key_pem
        .map(|value| snapshot(value, ScInputRole::PrivateKey))
        .transpose()?;
    inspection::inspect_sc_pem(&identity, trust.as_deref(), key.as_deref(), now)
        .map(PyScPemInspection)
        .map_err(error)
}

/// Read explicitly selected files once, returning an advisory immutable report.
#[pyfunction]
#[pyo3(signature = (identity_path, *, peer_trust_path=None, private_key_path=None, inspected_at_unix=None))]
fn inspect_sc_pem_files(
    identity_path: &Bound<'_, PyAny>,
    peer_trust_path: Option<&Bound<'_, PyAny>>,
    private_key_path: Option<&Bound<'_, PyAny>>,
    inspected_at_unix: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyScPemInspection> {
    let now = time(inspected_at_unix)?;
    let identity = path(identity_path)?;
    let trust = peer_trust_path.map(path).transpose()?;
    let key = private_key_path.map(path).transpose()?;
    identity_path
        .py()
        .detach(|| {
            inspection::inspect_sc_pem_files(&identity, trust.as_deref(), key.as_deref(), now)
        })
        .map(PyScPemInspection)
        .map_err(error)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyScPemInspection>()?;
    m.add_class::<PyScInspectedCertificate>()?;
    m.add_class::<PyScInspectionFinding>()?;
    m.add_class::<PyScExtensionInspection>()?;
    m.add_class::<PyScKeyFilePermissions>()?;
    m.add_function(wrap_pyfunction!(inspect_sc_pem, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_sc_pem_files, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::exceptions::PyBufferError;

    #[test]
    fn mutable_input_export_pins_length_until_the_bounded_snapshot_finishes() {
        Python::initialize();
        Python::attach(|py| {
            for value in [
                PyByteArray::new(py, b"abc"),
                py.eval(
                    c"type('Redirected', (bytearray,), {'__buffer__': lambda self, flags: memoryview(bytearray(b'other'))})(b'abc')",
                    None,
                    None,
                )
                .unwrap()
                .cast_into::<PyByteArray>()
                .unwrap(),
            ] {
            let input = PemInput::pin(value.as_any()).unwrap();
            let other = value.clone().unbind();
            // A different attached thread tries to enlarge the array between
            // acquiring the export and copying it, without a timing race.
            let resize = std::thread::spawn(move || {
                Python::attach(|py| {
                    other
                        .bind(py)
                        .resize(MAX_INPUT_BYTES + 1)
                        .unwrap_err()
                        .is_instance_of::<PyBufferError>(py)
                })
            });
            assert!(py.detach(|| resize.join().unwrap()));
            let mut remaining = 3;
            let snapshot = input
                .snapshot(&mut remaining, ScInputRole::Identity)
                .unwrap();
            assert_eq!(snapshot.as_ref(), b"abc");
            assert_eq!(remaining, 0);
            value.resize(4).unwrap();
            assert_eq!(snapshot.as_ref(), b"abc");
            }
        });
    }

    #[test]
    fn aggregate_snapshot_budget_and_error_paths_release_mutable_exports() {
        Python::initialize();
        Python::attach(|py| {
            let identity = PyByteArray::new(py, b"abc");
            let trust = PyByteArray::new(py, b"de");
            let key = PyByteArray::new(py, b"f");
            let mut remaining = 4;
            let snapshot = PemInput::pin(identity.as_any())
                .unwrap()
                .snapshot(&mut remaining, ScInputRole::Identity)
                .unwrap();
            assert_eq!(snapshot.len(), 3);
            assert_eq!(remaining, 1);
            let error = PemInput::pin(trust.as_any())
                .unwrap()
                .snapshot(&mut remaining, ScInputRole::PeerTrust)
                .unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
            assert!(error.to_string().contains("input_too_large"));
            assert_eq!(remaining, 1);
            // Rejection drops its export; a successful previous copy released
            // its export too, even while the owned snapshot remains alive.
            trust.resize(3).unwrap();
            identity.resize(4).unwrap();
            let last = PemInput::pin(key.as_any())
                .unwrap()
                .snapshot(&mut remaining, ScInputRole::PrivateKey)
                .unwrap();
            assert_eq!(last.as_ref(), b"f");
            assert_eq!(remaining, 0);
            key.resize(2).unwrap();
        });
    }
}
