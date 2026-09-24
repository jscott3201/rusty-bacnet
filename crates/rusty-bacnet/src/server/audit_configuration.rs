use super::*;

use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::BACnetObjectSelector;
use bacnet_types::enums::{AuditLevel, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::types::{PyBool, PyDict, PyInt, PyList};

use crate::types::PyObjectType;

fn monitored_object_selectors(
    value: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<Vec<BACnetObjectSelector>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let selectors = value
        .cast_exact::<PyList>()
        .map_err(|_| PyTypeError::new_err("monitored_objects must be a list or None"))?;
    selectors
        .iter()
        .enumerate()
        .map(|(index, selector)| {
            if selector.is_none() {
                Ok(BACnetObjectSelector::None)
            } else if let Ok(object) = selector.extract::<PyObjectIdentifier>() {
                Ok(BACnetObjectSelector::Object(object.to_rust()))
            } else if let Ok(kind) = selector.extract::<PyObjectType>() {
                Ok(BACnetObjectSelector::ObjectType(kind.to_rust()))
            } else {
                Err(PyTypeError::new_err(format!(
                    "monitored_objects[{index}] must be ObjectIdentifier, ObjectType, or None"
                )))
            }
        })
        .collect::<PyResult<Vec<_>>>()
        .map(Some)
}

fn priority_filter(value: Option<&Bound<'_, PyAny>>) -> PyResult<BACnetPriorityFilter> {
    let Some(value) = value else {
        return Ok(BACnetPriorityFilter::all());
    };
    if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(
            "audit_priority_filter must be an integer (not bool) or None",
        ));
    }
    let bits = value
        .extract::<u16>()
        .map_err(|_| PyValueError::new_err("audit_priority_filter must be in 0..=65535"))?;
    Ok(BACnetPriorityFilter::from_bits(bits))
}

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

pub(super) fn pending_audit_reporter_index(
    objects: &[Box<dyn BACnetObject + Send>],
    object_id: ObjectIdentifier,
) -> PyResult<usize> {
    let mut matches = objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.object_identifier() == object_id);
    let (index, object) = matches.next().ok_or_else(|| {
        PyValueError::new_err(format!(
            "no pending Audit Reporter object with instance {}",
            object_id.instance_number()
        ))
    })?;
    if matches.next().is_some() {
        return Err(PyValueError::new_err(format!(
            "duplicate pending Audit Reporter instance {}",
            object_id.instance_number()
        )));
    }
    if object.audit_reporter_internal().is_none() {
        return Err(PyValueError::new_err(
            "selected object does not support the Audit Reporter capability",
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
    /// Configure one through 64 registered target Reporters before start().
    /// Each dict owns one complete configuration; the entire list is validated
    /// before any pending object changes. A later valid call replaces the set.
    fn configure_audit_reporters(&mut self, reporters: &Bound<'_, PyAny>) -> PyResult<()> {
        let configs = parse_reporters(reporters)?;
        let identities = configs.iter().map(|value| value.identifier).collect();
        {
            let mut pending = self.lock_pending()?;
            self.check_forwarding_configuration()?;
            let indices = configs
                .iter()
                .map(|value| pending_audit_reporter_index(&pending, value.identifier))
                .collect::<PyResult<Vec<_>>>()?;
            for index in &indices {
                pending[*index]
                    .audit_reporter_authority_internal()
                    .ok_or_else(|| {
                        PyValueError::new_err("selected object lacks concrete Reporter authority")
                    })?
                    .validate_installation()
                    .map_err(to_py_err)?;
            }
            for (config, index) in configs.into_iter().zip(indices) {
                pending[index]
                    .configure_audit_reporter_internal(
                        config.level,
                        config.operations,
                        config.confirmed,
                        config.selectors,
                        config.priorities,
                        config.maximum_send_delay,
                    )
                    .map_err(to_py_err)?;
            }
        }
        self.audit_reporters = Some(server::AuditReportersConfig {
            reporters: identities,
        });
        Ok(())
    }

    /// Provision the Device-owned target recipient before startup. The input is
    /// copied; live changes use the Device property and notify both destinations.
    fn configure_audit_recipient(&mut self, recipient: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = crate::types::audit_recipient_from_py(recipient, "recipient")?;
        self.validate_audit_recipient_input(&value)?;
        self.check_forwarding_configuration()?;
        *self
            .audit_recipient
            .lock()
            .map_err(|_| PyRuntimeError::new_err("recipient lock poisoned"))? = Some(value);
        Ok(())
    }

    /// Add one direct B/IP (IPv4) Device binding before start(), using IPv4:port or six hex bytes.
    /// Duplicate Device identifiers are rejected, never overwritten. No routed bindings.
    #[pyo3(signature = (device_instance, address))]
    fn add_device_binding(
        &mut self,
        device_instance: &Bound<'_, PyAny>,
        address: &str,
    ) -> PyResult<()> {
        let device = instance_identifier(device_instance, "device_instance", ObjectType::DEVICE)?;
        let mac = crate::types::parse_address(address)?;
        {
            let _pending = self.lock_pending()?;
            self.check_forwarding_configuration()?;
        }
        if self.transport_type != "bip" {
            return Err(PyValueError::new_err(
                "device bindings require BACnetServer transport='bip' (IPv4)",
            ));
        }
        if mac.len() != 6 {
            return Err(PyValueError::new_err(
                "B/IP device binding address must encode exactly 6 bytes (IPv4 and port)",
            ));
        }
        let binding = server::DeviceBinding::local(device, mac)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
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

impl BACnetServer {
    pub(super) fn validate_audit_recipient_input(
        &self,
        recipient: &bacnet_types::constructed::BACnetRecipient,
    ) -> PyResult<()> {
        use bacnet_types::constructed::BACnetRecipient;
        match recipient {
            BACnetRecipient::Device(device) if device.object_type() == ObjectType::DEVICE
                && device.instance_number() < ObjectIdentifier::MAX_INSTANCE
                && device.instance_number() != self.device_instance => Ok(()),
            BACnetRecipient::Address(address) if self.transport_type == "bip"
                && server::valid_bip_audit_address(address)
                && address.mac_address.as_slice()[..4] != self.broadcast_address.parse::<std::net::Ipv4Addr>()
                    .map_err(|_| PyValueError::new_err("invalid configured broadcast address"))?.octets() => Ok(()),
            _ => Err(PyValueError::new_err("recipient must be a concrete remote Device or a supported direct unicast B/IP address")),
        }
    }
}

struct ReporterConfiguration {
    identifier: ObjectIdentifier,
    level: AuditLevel,
    operations: AuditOperationFlags,
    confirmed: bool,
    selectors: Option<Vec<BACnetObjectSelector>>,
    priorities: BACnetPriorityFilter,
    maximum_send_delay: Option<bacnet_objects::audit::AuditSendDelay>,
}

fn parse_reporters(value: &Bound<'_, PyAny>) -> PyResult<Vec<ReporterConfiguration>> {
    let values = value
        .cast_exact::<PyList>()
        .map_err(|_| PyTypeError::new_err("reporters must be a list of configuration dicts"))?;
    if !(1..=64).contains(&values.len()) {
        return Err(PyValueError::new_err(
            "configure one through 64 target Audit Reporters",
        ));
    }
    let mut result = Vec::with_capacity(values.len());
    for value in values.iter() {
        let config = value
            .cast_exact::<PyDict>()
            .map_err(|_| PyTypeError::new_err("each Reporter configuration must be a dict"))?;
        for key in config.keys() {
            if !matches!(
                key.extract::<&str>()?,
                "instance"
                    | "audit_level"
                    | "auditable_operations"
                    | "issue_confirmed_notifications"
                    | "monitored_objects"
                    | "audit_priority_filter"
                    | "maximum_send_delay"
            ) {
                return Err(PyValueError::new_err(
                    "unknown Reporter configuration field",
                ));
            }
        }
        let required = |name| {
            config
                .get_item(name)?
                .ok_or_else(|| PyValueError::new_err(format!("missing Reporter field: {name}")))
        };
        let identifier = instance_identifier(
            &required("instance")?,
            "instance",
            ObjectType::AUDIT_REPORTER,
        )?;
        if identifier.instance_number() == ObjectIdentifier::MAX_INSTANCE {
            return Err(PyValueError::new_err(
                "Reporter instance must be concrete (0..=4194302)",
            ));
        }
        let level = match required("audit_level")?.extract::<&str>()? {
            "none" => AuditLevel::NONE,
            "audit_config" => AuditLevel::AUDIT_CONFIG,
            "audit_all" => AuditLevel::AUDIT_ALL,
            _ => {
                return Err(PyValueError::new_err(
                    "audit_level must be 'none', 'audit_config', or 'audit_all'",
                ))
            }
        };
        let operations = required("auditable_operations")?;
        if operations.is_instance_of::<PyBool>() || !operations.is_instance_of::<PyInt>() {
            return Err(PyTypeError::new_err(
                "auditable_operations must be an integer (not bool)",
            ));
        }
        let bits = operations.extract::<u64>().map_err(|_| {
            PyValueError::new_err("auditable_operations must be in 0..=18446744073709551615")
        })?;
        let operations = AuditOperationFlags::from_bits(bits)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let confirmed = required("issue_confirmed_notifications")?;
        if !confirmed.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "issue_confirmed_notifications must be bool",
            ));
        }
        let selectors = config
            .get_item("monitored_objects")?
            .filter(|value| !value.is_none());
        let priorities = config
            .get_item("audit_priority_filter")?
            .filter(|value| !value.is_none());
        let maximum_send_delay = config
            .get_item("maximum_send_delay")?
            .filter(|v| !v.is_none())
            .map(|v| {
                if v.is_instance_of::<PyBool>() || !v.is_instance_of::<PyInt>() {
                    return Err(PyTypeError::new_err(
                        "maximum_send_delay must be an integer (not bool) or None",
                    ));
                }
                let seconds = v
                    .extract::<u32>()
                    .map_err(|_| PyValueError::new_err("maximum_send_delay must be in 0..=3600"))?;
                bacnet_objects::audit::AuditSendDelay::new(seconds)
                    .map_err(|_| PyValueError::new_err("maximum_send_delay must be in 0..=3600"))
            })
            .transpose()?;
        result.push(ReporterConfiguration {
            identifier,
            level,
            operations,
            confirmed: confirmed.extract()?,
            selectors: monitored_object_selectors(selectors.as_ref())?,
            priorities: priority_filter(priorities.as_ref())?,
            maximum_send_delay,
        });
    }
    result.sort_by_key(|value| value.identifier.instance_number());
    if result
        .windows(2)
        .any(|pair| pair[0].identifier == pair[1].identifier)
    {
        return Err(PyValueError::new_err(
            "duplicate target Audit Reporter instance",
        ));
    }
    Ok(result)
}
