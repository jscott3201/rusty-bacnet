//! Credential configuration and a read-only view of the native rights stage.

use super::super::*;
use crate::types::{
    AccessRightsValue, assigned_access_rights_from_py, authorization_exemptions_from_py,
};

#[pymethods]
impl BACnetServer {
    /// Add an Access Credential before starting the server.
    ///
    /// Each `assigned_access_rights` mapping requires a reference under that
    /// same key and an `enable` bool. A reference is a local ObjectIdentifier
    /// or a `(Device, object)` pair. Omitted or None assignments mean no rights.
    /// `authorization_exemptions` is a list of integer criterion numbers:
    /// omitted or None leaves the optional row absent; [] serves an empty row.
    /// The Rust setters validate both lists before anything is registered.
    #[pyo3(signature = (instance, name, *, assigned_access_rights=None, authorization_exemptions=None))]
    fn add_access_credential(
        &self,
        instance: u32,
        name: &str,
        assigned_access_rights: Option<Bound<'_, PyAny>>,
        authorization_exemptions: Option<Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let rights = assigned_access_rights
            .as_ref()
            .map(assigned_access_rights_from_py)
            .transpose()?
            .unwrap_or_default();
        let exemptions = authorization_exemptions
            .as_ref()
            .map(authorization_exemptions_from_py)
            .transpose()?;
        let mut object = AccessCredentialObject::new(instance, name).map_err(to_py_err)?;
        object
            .set_assigned_access_rights(rights)
            .map_err(to_py_err)?;
        object
            .set_authorization_exemptions(exemptions)
            .map_err(to_py_err)?;
        self.push_pending(Box::new(object))
    }

    /// Inspect the existing local rights stage (Clause 12.34.9.2).
    ///
    /// Returns an awaitable for a plain mapping with `decision`, `access_event`,
    /// `rule` and ordered `unresolved` evidence. Rule and assignment indices
    /// are one-based original positions. A grant may include unresolved rights.
    /// `granted` means this stage passed, not final access authorization;
    /// `exempt` applies only to this criterion (Clause 12.35.25).
    /// The nullable denial event is evidence, never a recorded Access_Event.
    /// No remote resolution, event/COV reporting or physical action occurs.
    /// Requires a started server; missing or wrong-type identifiers raise the
    /// native OBJECT/UNKNOWN_OBJECT protocol error.
    #[pyo3(signature = (credential, point))]
    fn evaluate_access_rights<'py>(
        &self,
        py: Python<'py>,
        credential: PyObjectIdentifier,
        point: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let credential = credential.to_rust();
        let point = point.to_rust();
        crate::py_async::future_into_py(py, async move {
            let db = {
                let guard = inner.lock().await;
                let server = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                server.database().clone()
            };
            let result = {
                let database = db.read().await;
                bacnet_objects::access_control::evaluate_access_rights(&database, credential, point)
            };
            // The owned result crosses the Python boundary only after both locks drop.
            result.map(AccessRightsValue).map_err(to_py_err)
        })
    }
}
