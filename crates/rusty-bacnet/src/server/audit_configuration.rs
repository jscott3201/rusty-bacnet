use super::*;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyBool, PyInt};

/// Owned pre-start configuration; request authorization never enters Python.
#[derive(Clone, Copy)]
pub(super) struct AuditNotificationSink {
    pub object_id: ObjectIdentifier,
    pub allow_all: bool,
}

impl AuditNotificationSink {
    pub(super) fn validate(&self, objects: &[Box<dyn BACnetObject + Send>]) -> PyResult<()> {
        let mut matches = objects
            .iter()
            .filter(|object| object.object_identifier() == self.object_id);
        let object = matches.next().ok_or_else(|| {
            PyValueError::new_err(format!(
                "no pending Audit Log object with instance {}",
                self.object_id.instance_number()
            ))
        })?;
        if matches.next().is_some() {
            return Err(PyValueError::new_err(format!(
                "duplicate pending Audit Log instance {}",
                self.object_id.instance_number()
            )));
        }
        if object.audit_log_storage_internal().is_none() {
            return Err(PyValueError::new_err(
                "selected Audit Log does not support Audit storage",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl BACnetServer {
    /// Select one registered Audit Log before start(), with an explicit static policy.
    ///
    /// policy must be 'deny_all' or 'allow_all'. Unconfigured servers deny receipt.
    /// This is transport admission only: payload identities remain peer-reported.
    /// Invalid configuration raises ValueError without replacing the previous choice.
    /// A running server raises RuntimeError; no runtime policy mutation is supported.
    #[pyo3(signature = (instance, *, policy))]
    fn configure_audit_notification_sink(
        &mut self,
        instance: &Bound<'_, PyAny>,
        policy: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let invalid_instance =
            || PyValueError::new_err("instance must be an integer in 0..=4194303 (not bool)");
        if instance.is_instance_of::<PyBool>() || !instance.is_instance_of::<PyInt>() {
            return Err(invalid_instance());
        }
        let instance = instance.extract::<u32>().map_err(|_| invalid_instance())?;
        let object_id = ObjectIdentifier::new(ObjectType::AUDIT_LOG, instance)
            .map_err(|_| invalid_instance())?;
        let invalid_policy = || PyValueError::new_err("policy must be 'deny_all' or 'allow_all'");
        let allow_all = match policy.extract::<&str>().map_err(|_| invalid_policy())? {
            "deny_all" => false,
            "allow_all" => true,
            _ => return Err(invalid_policy()),
        };
        let sink = AuditNotificationSink {
            object_id,
            allow_all,
        };
        {
            let objects = self.lock_pending()?;
            if self.started.load(Ordering::Acquire) {
                return Err(PyRuntimeError::new_err(
                    "cannot configure Audit receipt after start() — server is already running",
                ));
            }
            sink.validate(&objects)?;
        }
        self.audit_notification_sink = Some(sink);
        Ok(())
    }
}
