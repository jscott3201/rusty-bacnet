//! Shared validated config + identity/database helpers for Python endpoints.
//!
//! Constructors validate configuration values without I/O. Admitted async
//! preparation performs TLS/file/serial setup and reports setup errors through
//! the startup Future. No post-start configuration mutation is exposed.

use bacnet_types::error::RemoteError;
use std::net::Ipv4Addr;

use bacnet_endpoint::identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::audit::ObjectAuditPolicy;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::ObjectType;
use bacnet_types::enums::{AuditLevel, ErrorClass, ErrorCode, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::{PyErr, PyResult};

use crate::algorithm_keywords::AlgorithmKeywords;
use crate::errors::to_py_err;
use crate::group_members;
use crate::types::{PyReadAccessSpec, PySegmentation};

/// Parse an IPv4 string at construction time (ValueError before bind).
pub(crate) fn parse_ipv4(value: &str, field: &str) -> PyResult<Ipv4Addr> {
    value
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid {field}: {e}")))
}

/// Validate the server's read work limit (#1250): the result rows one
/// ReadProperty or ReadPropertyMultiple may expand, a Group's member rows
/// included, which the full server's ReadPropertyMultiple budget carries
/// (#2617).
///
/// Zero is a ValueError here, at construction, as the server's start would
/// refuse a budget of none.
pub(crate) fn parse_read_work_limit(read_work_limit: usize) -> PyResult<usize> {
    if read_work_limit == 0 {
        return Err(PyValueError::new_err(
            "read_work_limit must be greater than zero",
        ));
    }
    Ok(read_work_limit)
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

/// Map an optional segmentation override (default: NO_SEGMENTATION, the proven value).
pub(crate) fn parse_segmentation(segmentation: Option<PySegmentation>) -> Segmentation {
    segmentation
        .map(|s| s.to_rust())
        .unwrap_or(Segmentation::NO_SEGMENTATION)
}

/// Build the single validated DeviceIdentity (builder-time only).
///
/// All Encoding failures (instance range, receive ceiling, duplicate ports) map
/// to ValueError before any bind/dial. No post-start mutation exists. The
/// identity keeps its default service list: the full server lists the
/// services it executes in the Device's Protocol_Services_Supported
/// whatever the identity says (#2617).
pub(crate) fn build_identity(
    device_instance: u32,
    device_name: &str,
    vendor_id: u16,
    max_apdu: u16,
    segmentation: Segmentation,
    device_uuid: [u8; 16],
) -> PyResult<DeviceIdentity> {
    let identity = DeviceIdentity::new(device_instance, vendor_id)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity
        .with_max_apdu(max_apdu)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity.with_segmentation(segmentation);
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
/// parity: `ObjectDatabase::add` reports `Error::protocol(PROPERTY,
/// DUPLICATE_NAME)`, never an Encoding string). So does an object whose
/// identifier the Device, a port or another object already has (OBJECT /
/// OBJECT_IDENTIFIER_ALREADY_EXISTS, #2250).
pub(crate) fn build_database(
    identity: &DeviceIdentity,
    extra: Vec<Box<dyn BACnetObject>>,
) -> PyResult<ObjectDatabase> {
    build_database_with_extra(identity, extra).map_err(|e| match &e {
        bacnet_types::error::Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::DUPLICATE_NAME =>
        {
            PyValueError::new_err(format!("duplicate object name: {e}"))
        }
        bacnet_types::error::Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::OBJECT
                && *code == ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS =>
        {
            PyValueError::new_err(format!("duplicate object identifier: {e}"))
        }
        bacnet_types::error::Error::Encoding(message)
            if message.contains("duplicate object name") =>
        {
            PyValueError::new_err(e.to_string())
        }
        _ => to_py_err(e),
    })
}

// ---------------------------------------------------------------------------
// Rebuildable pending registrations, owned by the private lifecycle gate
// ---------------------------------------------------------------------------
// Registrations stay in the admitted worker until startup publishes or cleanup
// settles. Boxes are built fresh; failed/cancelled attempts restore the params
// only after releasing the corresponding resources.

/// The Value-object keywords a pending registration keeps for its rebuilds
/// (#1552, #1867): how Present_Value is written, whether Value_Source is
/// served, and the optional Audit policy.
#[derive(Clone, Debug)]
pub(crate) struct ValueKeywords {
    pub(crate) access: PresentValueAccess,
    pub(crate) value_source_tracking: bool,
    pub(crate) audit_policy: ObjectAuditPolicy,
}

/// Pending object parameters: one entry per pre-start `add_*` call.
///
/// `Clone` so a failed/cancelled start restores the exact registration set
/// for retry. Built into boxes at start time; the `add_*` call builds one
/// first, so a bad registration fails there. `profile` is the parsed
/// `tags`/`profile_location`/`profile_name` keywords (#1867), and
/// `parameters` the event algorithm parameter keywords (#2410).
#[derive(Clone, Debug)]
pub(crate) enum PendingObject {
    AnalogInput {
        instance: u32,
        name: String,
        units: u32,
        present_value: f32,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    AnalogValue {
        instance: u32,
        name: String,
        units: u32,
        value: ValueKeywords,
        present_value: Option<f32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    AnalogOutput {
        instance: u32,
        name: String,
        units: u32,
        present_value: Option<f32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    BinaryInput {
        instance: u32,
        name: String,
        polarity: bacnet_types::enums::Polarity,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    BinaryOutput {
        instance: u32,
        name: String,
        polarity: bacnet_types::enums::Polarity,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    BinaryValue {
        instance: u32,
        name: String,
        value: ValueKeywords,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    MultiStateInput {
        instance: u32,
        name: String,
        number_of_states: u32,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    MultiStateOutput {
        instance: u32,
        name: String,
        number_of_states: u32,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    MultiStateValue {
        instance: u32,
        name: String,
        number_of_states: u32,
        access: PresentValueAccess,
        value_source_tracking: bool,
        present_value: Option<u32>,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    },
    Group {
        instance: u32,
        name: String,
        members: Vec<ReadAccessSpecification>,
    },
    /// An endpoint's Audit Reporter, configured once here: peers can't
    /// write it through an endpoint (#2348).
    AuditReporter {
        instance: u32,
        name: String,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
    },
}

#[path = "pending_build.rs"]
mod pending_build;

/// Validate the fresh object before admission, then check the entire identity
/// and pending set under the same lock that seals registrations at startup.
pub(super) fn register_pending<S>(
    lifecycle: &super::lifecycle::Lifecycle<S, PendingObject>,
    identity: &DeviceIdentity,
    candidate: PendingObject,
) -> PyResult<()> {
    let (kind, instance, _) = candidate.identity();
    ObjectIdentifier::new_addressable(kind, instance).map_err(to_py_err)?;
    candidate.build()?;
    lifecycle
        .with_open_registrations(|objects| {
            let (_, _, name) = candidate.identity();
            let check = |other_kind, other_instance, other_name: &str| -> PyResult<()> {
                if (kind, instance) == (other_kind, other_instance) {
                    return Err(PyValueError::new_err(format!(
                        "duplicate object identifier: {kind:?} {instance}"
                    )));
                }
                if name == other_name {
                    return Err(PyValueError::new_err(format!(
                        "duplicate object name: {name}"
                    )));
                }
                Ok(())
            };
            check(ObjectType::DEVICE, identity.instance(), identity.name())?;
            for port in identity.network_ports() {
                check(ObjectType::NETWORK_PORT, port.instance, &port.object_name())?;
            }
            for object in objects.iter() {
                let (kind, instance, name) = object.identity();
                check(kind, instance, name)?;
            }
            objects.push(candidate);
            Ok(())
        })
        .ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "cannot add objects while endpoint is starting, running, or stopping",
            )
        })?
}

/// Builds boxes from borrowed params (params retained on error).
pub(crate) fn build_pending_boxes(
    params: &[PendingObject],
) -> PyResult<Vec<Box<dyn BACnetObject>>> {
    params.iter().map(PendingObject::build).collect()
}

pub(crate) fn lifecycle_error(error: super::lifecycle::LifecycleError<PyErr>) -> PyErr {
    match error {
        super::lifecycle::LifecycleError::AlreadyStarted => to_py_err(
            bacnet_types::error::Error::Encoding("endpoint already started".into()),
        ),
        super::lifecycle::LifecycleError::StartupCancelled => {
            pyo3::exceptions::PyRuntimeError::new_err("endpoint startup cancelled by close")
        }
        super::lifecycle::LifecycleError::Operation(error) => error,
        super::lifecycle::LifecycleError::WorkerEnded => pyo3::exceptions::PyRuntimeError::new_err(
            "endpoint lifecycle worker ended unexpectedly",
        ),
    }
}

/// Capture a Group's member specifications for validation at registration
/// and reconstruction on startup through the shared [`crate::group_members`] builder.
pub(crate) fn pending_group(
    instance: u32,
    name: &str,
    members: Option<Vec<PyReadAccessSpec>>,
) -> PendingObject {
    let members = group_members::members(members);
    PendingObject::Group {
        instance,
        name: name.to_string(),
        members,
    }
}

#[cfg(test)]
mod build_database_tests {
    use super::*;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_types::enums::EngineeringUnits;
    use pyo3::Python;

    fn point(instance: u32, name: &str) -> Box<dyn BACnetObject> {
        Box::new(AnalogInputObject::new(instance, name, EngineeringUnits::NO_UNITS).unwrap())
    }

    /// Both duplicate kinds surface as ValueError, named for what clashed.
    #[test]
    fn duplicate_names_and_identifiers_are_value_errors() {
        Python::initialize();
        let identity = DeviceIdentity::new(1001, 42).unwrap();
        for (extra, expected) in [
            (
                vec![point(1, "same"), point(2, "same")],
                "duplicate object name",
            ),
            (
                vec![point(9, "ai-9-a"), point(9, "ai-9-b")],
                "duplicate object identifier",
            ),
        ] {
            let Err(error) = build_database(&identity, extra) else {
                panic!("{expected}: the database must be refused");
            };
            Python::attach(|py| {
                assert!(error.is_instance_of::<PyValueError>(py), "{expected}");
                assert!(error.to_string().contains(expected), "{error}");
            });
        }
    }
}
