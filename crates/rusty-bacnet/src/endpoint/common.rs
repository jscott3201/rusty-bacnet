//! Shared validated config + identity/database helpers for Python endpoints.
//!
//! Constructors validate configuration values without I/O. Admitted async
//! preparation performs TLS/file/serial setup and reports setup errors through
//! the startup Future. No post-start configuration mutation is exposed.

use bacnet_types::error::RemoteError;
use std::net::Ipv4Addr;

use bacnet_endpoint::identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::audit::{AuditReporterObject, ObjectAuditPolicy};
use bacnet_objects::binary::{BinaryInputObject, BinaryValueObject};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::{
    AuditLevel, EngineeringUnits, ErrorClass, ErrorCode, Segmentation, ServiceSupported,
};
use pyo3::exceptions::PyValueError;
use pyo3::{PyErr, PyResult};

use crate::errors::to_py_err;
use crate::group_members;
use crate::object_options::apply_profile;
use crate::types::{PyReadAccessSpec, PySegmentation};

/// Parse an IPv4 string at construction time (ValueError before bind).
pub(crate) fn parse_ipv4(value: &str, field: &str) -> PyResult<Ipv4Addr> {
    value
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid {field}: {e}")))
}

/// Validate the server role's read work limit (#1250): the result rows one
/// ReadProperty may expand, a Group's member rows included.
///
/// Zero is a ValueError here, at construction, so the native
/// `EndpointSession::new` refusal of a zero limit is never reached at start.
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
/// All Encoding failures (instance range, receive ceiling, duplicate ports) map
/// to ValueError before any bind/dial. No post-start mutation exists.
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
/// `tags`/`profile_location`/`profile_name` keywords (#1867).
#[derive(Clone, Debug)]
pub(crate) enum PendingObject {
    AnalogInput {
        instance: u32,
        name: String,
        units: u32,
        present_value: f32,
        profile: Option<ObjectProfile>,
    },
    AnalogValue {
        instance: u32,
        name: String,
        units: u32,
        value: ValueKeywords,
        profile: Option<ObjectProfile>,
    },
    BinaryInput {
        instance: u32,
        name: String,
        profile: Option<ObjectProfile>,
    },
    BinaryValue {
        instance: u32,
        name: String,
        value: ValueKeywords,
        profile: Option<ObjectProfile>,
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

impl PendingObject {
    /// Builds the object, the same way at registration and at each start.
    pub(crate) fn build(&self) -> PyResult<Box<dyn BACnetObject>> {
        match self {
            Self::AnalogInput {
                instance,
                name,
                units,
                present_value,
                profile,
            } => {
                let mut object =
                    AnalogInputObject::new(*instance, name, EngineeringUnits::from_raw(*units))
                        .map_err(to_py_err)?;
                object.set_present_value(*present_value);
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::AnalogValue {
                instance,
                name,
                units,
                value,
                profile,
            } => {
                let mut object = AnalogValueObject::with_access(
                    *instance,
                    name,
                    EngineeringUnits::from_raw(*units),
                    value.access,
                )
                .map_err(to_py_err)?;
                object.set_audit_policy(value.audit_policy);
                object.set_value_source_tracking(value.value_source_tracking);
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::BinaryInput {
                instance,
                name,
                profile,
            } => {
                let mut object = BinaryInputObject::new(*instance, name).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::BinaryValue {
                instance,
                name,
                value,
                profile,
            } => {
                let mut object = BinaryValueObject::with_access(*instance, name, value.access)
                    .map_err(to_py_err)?;
                object.set_audit_policy(value.audit_policy);
                object.set_value_source_tracking(value.value_source_tracking);
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::Group {
                instance,
                name,
                members,
            } => Ok(Box::new(group_members::group(*instance, name, members)?)),
            Self::AuditReporter {
                instance,
                name,
                level,
                operations,
                confirmed,
            } => {
                let mut object = AuditReporterObject::new(*instance, name).map_err(to_py_err)?;
                object
                    .audit_reporter_mut()
                    .ok_or_else(|| PyValueError::new_err("Audit Reporter lacks its capability"))?
                    .configure(
                        *level,
                        *operations,
                        *confirmed,
                        None,
                        BACnetPriorityFilter::all(),
                        None,
                    )
                    .map_err(to_py_err)?;
                Ok(Box::new(object))
            }
        }
    }
}

/// Builds boxes from borrowed params (params retained on error).
pub(crate) fn build_pending_boxes(
    params: &[PendingObject],
) -> PyResult<Vec<Box<dyn BACnetObject>>> {
    params.iter().map(PendingObject::build).collect()
}

/// Session cleanup is awaited by the private lifecycle worker, never its waiter.
impl<T: bacnet_transport::port::TransportPort + 'static> super::lifecycle::Session
    for bacnet_endpoint::session::EndpointSession<T>
{
    type Error = PyErr;

    async fn start(&mut self) -> PyResult<()> {
        if let Err(error) = self.start().await {
            // Start errors before role setup leave no spawned transport workers;
            // role-setup errors already clean up. Settle Running before dropping.
            if self.is_running() {
                let _ = self.stop().await;
            }
            return Err(to_py_err(error));
        }
        Ok(())
    }

    async fn stop(&mut self) -> PyResult<()> {
        self.stop().await.map(|_| ()).map_err(to_py_err)
    }
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

/// Validate an `add_group` call and keep its params for the pending set.
///
/// `members` takes the `read_property_multiple` spec shape; the shared
/// [`crate::group_members`] conversion checks it the way `BACnetServer`'s
/// `add_group` does.
pub(crate) fn pending_group(
    instance: u32,
    name: &str,
    members: Option<Vec<PyReadAccessSpec>>,
) -> PyResult<PendingObject> {
    let members = group_members::members(members);
    group_members::group(instance, name, &members)?;
    Ok(PendingObject::Group {
        instance,
        name: name.to_string(),
        members,
    })
}

#[cfg(test)]
mod build_database_tests {
    use super::*;
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
