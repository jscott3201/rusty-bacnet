//! Shared validated config + identity/database helpers for Python endpoints.
//!
//! HubConfig-style: constructors validate everything before bind/dial; the
//! async start path only reuses owned values. No post-start owner mutation.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::audit::ObjectAuditPolicy;
use bacnet_objects::binary::{BinaryInputObject, BinaryValueObject};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, Segmentation, ServiceSupported};
use pyo3::exceptions::PyValueError;
use pyo3::PyResult;

use crate::errors::to_py_err;
use crate::types::PySegmentation;

/// Parse an IPv4 string at construction time (ValueError before bind).
pub(crate) fn parse_ipv4(value: &str, field: &str) -> PyResult<Ipv4Addr> {
    value
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid {field}: {e}")))
}

/// Validate the single durable device UUID (one source for transport+identity).
///
/// `required`: SC owners require a nonzero 16-byte UUID; BIP/MS/TP allow
/// `None` (zeros) for UUID-less operation.
pub(crate) fn parse_device_uuid(
    value: Option<Vec<u8>>,
    required: bool,
    field: &str,
) -> PyResult<[u8; 16]> {
    match value {
        None => {
            if required {
                return Err(PyValueError::new_err(format!("{field} is required")));
            }
            Ok([0; 16])
        }
        Some(bytes) => {
            let uuid: [u8; 16] = bytes
                .try_into()
                .map_err(|_| PyValueError::new_err(format!("{field} must be exactly 16 bytes")))?;
            if required && uuid == [0; 16] {
                return Err(PyValueError::new_err(format!(
                    "{field} must not be all zero"
                )));
            }
            Ok(uuid)
        }
    }
}

/// Map a services override to native bits (default: READ_PROPERTY only).
///
/// The endpoint responder executes ReadProperty (+Reject/Abort); advertising
/// a wider profile without composing the full server breaks the I-Am vs
/// behavior matrix (see `DeviceIdentity` docs).
pub(crate) fn parse_services(services: Option<Vec<u8>>) -> Vec<ServiceSupported> {
    match services {
        None => vec![ServiceSupported::READ_PROPERTY],
        Some(bits) => bits.into_iter().map(ServiceSupported::from_raw).collect(),
    }
}

/// Map an optional segmentation override (default: NONE, the proven value).
pub(crate) fn parse_segmentation(segmentation: Option<PySegmentation>) -> Segmentation {
    segmentation
        .map(|s| s.to_rust())
        .unwrap_or(Segmentation::NONE)
}

/// Build the single validated DeviceIdentity (builder-time only).
///
/// All Encoding failures (instance range, APDU table, duplicate ports) map
/// to ValueError before any bind/dial. No post-start mutation exists.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_identity(
    device_instance: u32,
    device_name: &str,
    vendor_id: u16,
    max_apdu: u16,
    segmentation: Segmentation,
    services: &[ServiceSupported],
    device_uuid: [u8; 16],
) -> PyResult<DeviceIdentity> {
    let identity = DeviceIdentity::new(device_instance, vendor_id)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity
        .with_max_apdu(max_apdu)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity.with_segmentation(segmentation);
    let identity = identity.with_services(services);
    let identity = identity.with_device_uuid(device_uuid);
    let identity = identity.with_name(device_name.to_string());
    Ok(identity)
}

/// Attach one Network-Port entry for BIP (ValueError on duplicates/range).
pub(crate) fn with_bip_port(
    identity: DeviceIdentity,
    instance: u32,
    network_number: u32,
    ip: Ipv4Addr,
    port: u16,
) -> PyResult<DeviceIdentity> {
    identity
        .with_bip_port(instance, network_number, ip, port)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Attach one Network-Port entry for SC (ValueError on duplicates/range).
pub(crate) fn with_sc_port(
    identity: DeviceIdentity,
    instance: u32,
    network_number: u32,
    vmac: [u8; 6],
) -> PyResult<DeviceIdentity> {
    identity
        .with_sc_port(instance, network_number, vmac)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Build the database from the single identity plus pending objects.
///
/// Truth direction stays identity-first: Device Object_List is seeded with
/// Device + ports + extras upfront. Duplicate names map to ValueError (server
/// parity: `ObjectDatabase::add` reports `Error::Protocol { OBJECT,
/// DUPLICATE_NAME }`, never an Encoding string).
pub(crate) fn build_database(
    identity: &DeviceIdentity,
    extra: Vec<Box<dyn BACnetObject>>,
) -> PyResult<ObjectDatabase> {
    build_database_with_extra(identity, extra).map_err(|e| match &e {
        bacnet_types::error::Error::Protocol { class, code }
            if *class == u32::from(ErrorClass::OBJECT.to_raw())
                && *code == u32::from(ErrorCode::DUPLICATE_NAME.to_raw()) =>
        {
            PyValueError::new_err(format!("duplicate object name: {e}"))
        }
        bacnet_types::error::Error::Encoding(message)
            if message.contains("duplicate object name") =>
        {
            PyValueError::new_err(e.to_string())
        }
        _ => to_py_err(e).into(),
    })
}

// ---------------------------------------------------------------------------
// Pending-registration seam (mirrors BACnetServer: std Mutex + started flag)
// ---------------------------------------------------------------------------
// RUN-1: registrations are stored as rebuildable params (not built boxes) so
// a failed/cancelled start can restore them and retry. Boxes are built fresh
// from these params on every start attempt; the params survive any drop of
// the start future, while built boxes are dropped with it.

/// Pending object parameters: one entry per pre-start `add_*` call.
///
/// `Clone` so a failed/cancelled start restores the exact registration set
/// for retry. Built into boxes at start time (after add-time validation).
#[derive(Clone, Debug)]
pub(crate) enum PendingObject {
    AnalogInput {
        instance: u32,
        name: String,
        units: u32,
        present_value: f32,
    },
    AnalogValue {
        audit_policy: ObjectAuditPolicy,
        instance: u32,
        name: String,
        units: u32,
    },
    BinaryInput {
        instance: u32,
        name: String,
    },
    BinaryValue {
        audit_policy: ObjectAuditPolicy,
        instance: u32,
        name: String,
    },
}

impl PendingObject {
    /// Builds the object (same constructors as the add-time validation).
    pub(crate) fn build(&self) -> PyResult<Box<dyn BACnetObject>> {
        match self {
            Self::AnalogInput {
                instance,
                name,
                units,
                present_value,
            } => make_analog_input(*instance, name, *units, *present_value),
            Self::AnalogValue {
                audit_policy,
                instance,
                name,
                units,
            } => make_analog_value(*instance, name, *units, *audit_policy),
            Self::BinaryInput { instance, name } => make_binary_input(*instance, name),
            Self::BinaryValue {
                instance,
                name,
                audit_policy,
            } => make_binary_value(*instance, name, *audit_policy),
        }
    }
}

/// Builds boxes from borrowed params (params retained on error).
pub(crate) fn build_pending_boxes(
    params: &[PendingObject],
) -> PyResult<Vec<Box<dyn BACnetObject>>> {
    params.iter().map(PendingObject::build).collect()
}

/// Drop-guard restoring drained params on every Err/cancel path.
///
/// Moved into the start future; callers borrow until success, then disarm
/// via `take()`. Any Err return — and future cancellation (drop) — puts the
/// drained vector back at the front, ahead of concurrent adds.
pub(crate) struct PendingRestoreGuard {
    pending: Arc<std::sync::Mutex<Vec<PendingObject>>>,
    objects: Option<Vec<PendingObject>>,
}

impl PendingRestoreGuard {
    pub(crate) fn new(
        pending: Arc<std::sync::Mutex<Vec<PendingObject>>>,
        objects: Vec<PendingObject>,
    ) -> Self {
        Self {
            pending,
            objects: Some(objects),
        }
    }

    /// Borrows the drained params (retained on error).
    pub(crate) fn objects(&self) -> &[PendingObject] {
        self.objects.as_deref().unwrap_or(&[])
    }

    /// Disarms after success (params consumed into the running session).
    pub(crate) fn take(&mut self) -> Option<Vec<PendingObject>> {
        self.objects.take()
    }
}

impl Drop for PendingRestoreGuard {
    fn drop(&mut self) {
        if let Some(objs) = self.objects.take() {
            if objs.is_empty() {
                return;
            }
            let mut guard = match self.pending.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            // Drained registrations predate concurrent adds: front-insert.
            let mut restored = objs;
            restored.extend(guard.drain(..));
            *guard = restored;
        }
    }
}

/// Create a pending Analog Input object (validation before start).
pub(crate) fn make_analog_input(
    instance: u32,
    name: &str,
    units: u32,
    present_value: f32,
) -> PyResult<Box<dyn BACnetObject>> {
    let mut object = AnalogInputObject::new(instance, name, units).map_err(to_py_err)?;
    object.set_present_value(present_value);
    Ok(Box::new(object))
}

/// Create a pending Analog Value object.
pub(crate) fn make_analog_value(
    instance: u32,
    name: &str,
    units: u32,
    audit_policy: ObjectAuditPolicy,
) -> PyResult<Box<dyn BACnetObject>> {
    let mut object = AnalogValueObject::new(instance, name, units).map_err(to_py_err)?;
    object.set_audit_policy(audit_policy);
    Ok(Box::new(object))
}

/// Create a pending Binary Input object.
pub(crate) fn make_binary_input(instance: u32, name: &str) -> PyResult<Box<dyn BACnetObject>> {
    let object = BinaryInputObject::new(instance, name).map_err(to_py_err)?;
    Ok(Box::new(object))
}

/// Create a pending Binary Value object.
pub(crate) fn make_binary_value(
    instance: u32,
    name: &str,
    audit_policy: ObjectAuditPolicy,
) -> PyResult<Box<dyn BACnetObject>> {
    let mut object = BinaryValueObject::new(instance, name).map_err(to_py_err)?;
    object.set_audit_policy(audit_policy);
    Ok(Box::new(object))
}
