//! Frozen Python copies of the shared Rust certificate interpretation.

use bacnet_transport::sc::ScHubPeerInfo;
use bacnet_transport::sc_tls::{
    ScCertificateMetadata, ScHandshakeKind, ScPeerCertificate, ScUntrustedPeerInfo,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};

/// Untrusted historical evidence from one rejected outbound TLS attempt.
#[pyclass(name = "ScUntrustedPeerInfo", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScUntrustedPeerInfo(pub(crate) ScUntrustedPeerInfo);

#[pymethods]
impl PyScUntrustedPeerInfo {
    #[getter]
    fn peer_address(&self) -> String {
        self.0.peer_address().to_string()
    }
    #[getter]
    fn server_name(&self) -> &str {
        self.0.server_name()
    }
    #[getter]
    fn certificates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        certificate_tuple(py, self.0.certificates())
    }
    fn __repr__(&self) -> String {
        format!(
            "ScUntrustedPeerInfo(certificate_count={})",
            self.0.certificates().len()
        )
    }
}

fn certificate_tuple<'py>(
    py: Python<'py>,
    certificates: &[ScPeerCertificate],
) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        certificates
            .iter()
            .map(|cert| Py::new(py, PyScPeerCertificate(cert.clone())))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// Historical observation of an accepted active Hub. Owns data only.
#[pyclass(name = "ScPeerInfo", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScPeerInfo(pub(crate) ScHubPeerInfo);

#[pymethods]
impl PyScPeerInfo {
    #[getter]
    fn connection_id(&self) -> u64 {
        self.0.tls().connection_id()
    }
    #[getter]
    fn observed_at_unix(&self) -> f64 {
        match self.0.observed_at().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        }
    }
    #[getter]
    fn peer_address(&self) -> String {
        self.0.tls().peer_address().to_string()
    }
    #[getter]
    fn server_name(&self) -> &str {
        self.0.tls().server_name()
    }
    #[getter]
    fn handshake_kind(&self) -> &'static str {
        match self.0.tls().handshake_kind() {
            ScHandshakeKind::Full => "full",
            ScHandshakeKind::Resumed => "resumed",
            ScHandshakeKind::Unknown => "unknown",
        }
    }
    #[getter]
    fn protocol_version(&self) -> Option<&str> {
        self.0.tls().protocol_version()
    }
    #[getter]
    fn cipher_suite(&self) -> Option<&str> {
        self.0.tls().cipher_suite()
    }
    #[getter]
    fn leaf_sha256<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.0.tls().leaf_sha256())
    }
    #[getter]
    fn certificates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        certificate_tuple(py, self.0.tls().certificates())
    }
    fn __repr__(&self) -> String {
        format!(
            "ScPeerInfo(connection_id={}, handshake_kind='{}', certificate_count={})",
            self.connection_id(),
            self.handshake_kind(),
            self.0.tls().certificates().len()
        )
    }
}

/// Original DER with optional display metadata; not a validation verdict.
#[pyclass(name = "ScPeerCertificate", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScPeerCertificate(ScPeerCertificate);

#[pymethods]
impl PyScPeerCertificate {
    #[getter]
    fn der<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.0.der())
    }
    #[getter]
    fn metadata(&self) -> Option<PyScCertificateMetadata> {
        self.0.metadata().cloned().map(PyScCertificateMetadata)
    }
    #[getter]
    fn inspection_error(&self) -> Option<&'static str> {
        self.0
            .metadata()
            .is_none()
            .then_some("certificate_parse_failed")
    }
    fn __repr__(&self) -> String {
        format!(
            "ScPeerCertificate(der_len={}, metadata_available={})",
            self.0.der().len(),
            self.0.metadata().is_some()
        )
    }
}

/// Bounded ASCII-escaped names/OIDs and signed Unix validity dates.
#[pyclass(name = "ScCertificateMetadata", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScCertificateMetadata(pub(crate) ScCertificateMetadata);

#[pymethods]
impl PyScCertificateMetadata {
    #[getter]
    fn subject(&self) -> &str {
        self.0.subject()
    }
    #[getter]
    fn issuer(&self) -> &str {
        self.0.issuer()
    }
    #[getter]
    fn not_before_unix(&self) -> i64 {
        self.0.not_before_unix()
    }
    #[getter]
    fn not_after_unix(&self) -> i64 {
        self.0.not_after_unix()
    }
    #[getter]
    fn public_key_algorithm_oid(&self) -> &str {
        self.0.public_key_algorithm_oid()
    }
    #[getter]
    fn signature_algorithm_oid(&self) -> &str {
        self.0.signature_algorithm_oid()
    }
    #[getter]
    fn text_truncated(&self) -> bool {
        self.0.text_truncated()
    }
    fn __repr__(&self) -> &'static str {
        "ScCertificateMetadata(...)"
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyScUntrustedPeerInfo>()?;
    m.add_class::<PyScPeerInfo>()?;
    m.add_class::<PyScPeerCertificate>()?;
    m.add_class::<PyScCertificateMetadata>()?;
    Ok(())
}
