use super::*;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyBool, PyInt};

fn instance_identifier(
    instance: &Bound<'_, PyAny>,
    name: &str,
    object_type: ObjectType,
) -> PyResult<ObjectIdentifier> {
    let invalid = || {
        PyValueError::new_err(format!(
            "{name} must be an integer in 0..=4194303 (not bool)"
        ))
    };
    if instance.is_instance_of::<PyBool>() || !instance.is_instance_of::<PyInt>() {
        return Err(invalid());
    }
    let instance = instance.extract::<u32>().map_err(|_| invalid())?;
    ObjectIdentifier::new(object_type, instance).map_err(|_| invalid())
}

pub(super) fn pending_audit_log_index(
    objects: &[Box<dyn BACnetObject + Send>],
    object_id: ObjectIdentifier,
) -> PyResult<usize> {
    let mut matches = objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.object_identifier() == object_id);
    let (index, object) = matches.next().ok_or_else(|| {
        PyValueError::new_err(format!(
            "no pending Audit Log object with instance {}",
            object_id.instance_number()
        ))
    })?;
    if matches.next().is_some() {
        return Err(PyValueError::new_err(format!(
            "duplicate pending Audit Log instance {}",
            object_id.instance_number()
        )));
    }
    if object.audit_log_storage_internal().is_none() {
        return Err(PyValueError::new_err(
            "selected Audit Log does not support Audit storage",
        ));
    }
    Ok(index)
}

/// Owned pre-start configuration; request authorization never enters Python.
#[derive(Clone, Copy)]
pub(super) struct AuditNotificationSink {
    pub object_id: ObjectIdentifier,
    pub allow_all: bool,
}

impl AuditNotificationSink {
    pub(super) fn validate(&self, objects: &[Box<dyn BACnetObject + Send>]) -> PyResult<()> {
        pending_audit_log_index(objects, self.object_id).map(|_| ())
    }
}

impl BACnetServer {
    fn check_forwarding_configuration(&self) -> PyResult<()> {
        if self
            .forwarding_configuration_started
            .load(Ordering::Acquire)
            || self.started.load(Ordering::Acquire)
        {
            return Err(PyRuntimeError::new_err(
                "cannot configure bindings or Audit parents after start() has consumed configuration",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl BACnetServer {
    /// Add one direct configured Device binding before start(), using the client address grammar.
    /// Duplicate Device identifiers are rejected, never overwritten. No routed bindings.
    #[pyo3(signature = (device_instance, address))]
    fn add_device_binding(
        &mut self,
        device_instance: &Bound<'_, PyAny>,
        address: &str,
    ) -> PyResult<()> {
        let device = instance_identifier(device_instance, "device_instance", ObjectType::DEVICE)?;
        let mac = crate::types::parse_address(address)?;
        let binding = server::DeviceBinding::local(device, mac)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        {
            let _pending = self.lock_pending()?;
            self.check_forwarding_configuration()?;
        }
        if self.device_bindings.contains_key(&device.instance_number()) {
            return Err(PyValueError::new_err(
                "duplicate configured Device identifier",
            ));
        }
        self.device_bindings
            .insert(device.instance_number(), binding);
        Ok(())
    }

    /// Set a registered Audit Log's parent before start(). Valid calls replace the previous parent.
    /// Invalid/missing/duplicate local instances leave the registered object unchanged.
    #[pyo3(signature = (instance, *, parent_device_instance, parent_audit_log_instance))]
    fn configure_audit_log_parent(
        &self,
        instance: &Bound<'_, PyAny>,
        parent_device_instance: &Bound<'_, PyAny>,
        parent_audit_log_instance: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let object_id = instance_identifier(instance, "instance", ObjectType::AUDIT_LOG)?;
        let device = instance_identifier(
            parent_device_instance,
            "parent_device_instance",
            ObjectType::DEVICE,
        )?;
        let parent_log = instance_identifier(
            parent_audit_log_instance,
            "parent_audit_log_instance",
            ObjectType::AUDIT_LOG,
        )?;
        if device.instance_number() == self.device_instance {
            return Err(PyValueError::new_err("parent Device must be remote"));
        }
        let mut pending = self.lock_pending()?;
        self.check_forwarding_configuration()?;
        let index = pending_audit_log_index(&pending, object_id)?;
        pending[index]
            .set_audit_log_parent_internal(BACnetDeviceObjectReference {
                device_identifier: Some(device),
                object_identifier: parent_log,
            })
            .map_err(to_py_err)
    }

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
        let object_id = instance_identifier(instance, "instance", ObjectType::AUDIT_LOG)?;
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
