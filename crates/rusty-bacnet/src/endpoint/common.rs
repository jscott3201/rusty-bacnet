//! Shared validated config + identity/database helpers for Python endpoints.
//!
//! Constructors validate configuration values without I/O. Admitted async
//! preparation performs TLS/file/serial setup and reports setup errors through
//! the startup Future. No post-start configuration mutation is exposed.

use bacnet_types::error::RemoteError;
use std::net::Ipv4Addr;

use bacnet_endpoint::identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::object_profile::FileTagsPersistence;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::ObjectType;
use bacnet_types::enums::{AuditLevel, ErrorClass, ErrorCode, Segmentation};
use pyo3::exceptions::PyValueError;
use pyo3::{PyErr, PyResult};

use crate::errors::to_py_err;
use crate::group_members;
use crate::point_registration::{Registration, Storage, admit};
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

/// Map an optional segmentation override (default: SEGMENTED_BOTH, the
/// identity's default, which both of the endpoint's roles follow, #2617).
pub(crate) fn parse_segmentation(segmentation: Option<PySegmentation>) -> Segmentation {
    segmentation
        .map(|s| s.to_rust())
        .unwrap_or(Segmentation::SEGMENTED_BOTH)
}

/// Build the single validated DeviceIdentity (builder-time only).
///
/// All Encoding failures (instance range, receive ceiling, segmentation and
/// its segment timeout, duplicate ports) map to ValueError before any
/// bind/dial. No post-start mutation exists. The identity keeps its default
/// service list: the full server lists the services it executes in the
/// Device's Protocol_Services_Supported whatever the identity says (#2617).
pub(crate) fn build_identity(
    device_instance: u32,
    device_name: &str,
    vendor_id: u16,
    max_apdu: u16,
    segmentation: Segmentation,
    apdu_segment_timeout_ms: u64,
    device_uuid: [u8; 16],
) -> PyResult<DeviceIdentity> {
    let identity = DeviceIdentity::new(device_instance, vendor_id)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity
        .with_max_apdu(max_apdu)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let identity = identity
        .with_segmentation(segmentation)
        .with_apdu_segment_timeout_ms(apdu_segment_timeout_ms);
    // An unknown mode, or a zero segment timeout while segmenting, is the
    // identity's own refusal; four segment timeouts must also be a deadline
    // the clock can hold, as `BACnetServer` checks (#2617).
    identity
        .validate()
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let representable = apdu_segment_timeout_ms
        .checked_mul(4)
        .and_then(|ms| std::time::Instant::now().checked_add(std::time::Duration::from_millis(ms)));
    if representable.is_none() {
        return Err(PyValueError::new_err(
            "apdu_segment_timeout_ms is too large: four of it must fit the clock",
        ));
    }
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
/// OBJECT_IDENTIFIER_ALREADY_EXISTS, #2250). With `device_storage`, the
/// Device keeps what peers write to it in that file (#2730); a file that
/// can't be read or loaded raises, naming the path.
pub(crate) fn build_database(
    identity: &DeviceIdentity,
    extra: Vec<Box<dyn BACnetObject>>,
    device_storage: Option<&FileTagsPersistence>,
) -> PyResult<ObjectDatabase> {
    let built = match device_storage {
        Some(storage) => bacnet_objects::device_identity::build_database_with_tags_persistence(
            identity,
            std::sync::Arc::new(storage.clone()),
            extra,
        ),
        None => build_database_with_extra(identity, extra),
    };
    built.map_err(|e| match &e {
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
        _ => to_py_err(e),
    })
}

// ---------------------------------------------------------------------------
// Rebuildable pending registrations, owned by the private lifecycle gate
// ---------------------------------------------------------------------------
// Registrations stay in the admitted worker until startup publishes or cleanup
// settles. Boxes are built fresh; failed/cancelled attempts restore the params
// only after releasing the corresponding resources.

/// Pending object parameters: one entry per pre-start `add_*` call.
///
/// `Clone` so a failed/cancelled start restores the exact registration set
/// for retry. Built into boxes at start time; the `add_*` call builds one
/// first, so a bad registration fails there. A point is the shape
/// `BACnetServer` registers and builds too (#2711, [`crate::point_registration`]).
#[derive(Clone, Debug)]
pub(crate) enum PendingObject {
    /// One of the nine point builders' registrations.
    Point(Registration),
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

mod pending_build;

/// What a call that needs the running session raises before start and
/// after close (#2819): `BacnetStateError`, `state` `"not_running"`, since
/// an endpoint starts again after its close.
pub(crate) fn not_running(message: &str) -> PyErr {
    crate::errors::lifecycle_error(bacnet_types::error::Lifecycle::NotRunning, message)
}

/// Admit a registration under the same lock that seals registrations at
/// startup, against the Device, its declared Network Ports and the pending
/// set, then keep it. Every kind passes through [`admit`], so a point is
/// checked as `BACnetServer` checks it, in the same order (#2711). The
/// object built here only proves the registration; start builds it again.
pub(super) fn register_pending<S>(
    lifecycle: &super::lifecycle::Lifecycle<S, PendingObject>,
    identity: &DeviceIdentity,
    candidate: PendingObject,
) -> PyResult<()> {
    let ports: Vec<(u32, String)> = identity
        .network_ports()
        .iter()
        .map(|port| (port.instance, port.object_name()))
        .collect();
    lifecycle
        .with_open_registrations(|objects| {
            let device = (ObjectType::DEVICE, identity.instance(), identity.name());
            let ports = ports
                .iter()
                .map(|(instance, name)| (ObjectType::NETWORK_PORT, *instance, name.as_str()));
            let kept = objects.iter().map(PendingObject::identity);
            admit(
                candidate.identity(),
                std::iter::once(device).chain(ports).chain(kept),
                || candidate.check(),
                || candidate.build(),
            )?;
            objects.push(candidate);
            Ok(())
        })
        .ok_or_else(|| {
            // Registrations are sealed from the start until the close ends:
            // a call only an unstarted endpoint takes (#2819).
            crate::errors::lifecycle_error(
                bacnet_types::error::Lifecycle::AlreadyStarted,
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

/// What the endpoint owner's start or close failed with: a second start
/// (`BacnetStateError`), a start that a close cancelled, the start's or
/// stop's own error, or a worker that ended (`RuntimeError`).
pub(crate) fn owner_error(error: super::lifecycle::LifecycleError<PyErr>) -> PyErr {
    match error {
        // A lifecycle refusal: BacnetStateError, `state` already_started
        // (#1833).
        super::lifecycle::LifecycleError::AlreadyStarted => {
            to_py_err(bacnet_types::error::Error::state(
                bacnet_types::error::Lifecycle::AlreadyStarted,
                "endpoint already started",
            ))
        }
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
    use pyo3::prelude::*;

    fn point(instance: u32, name: &str) -> Box<dyn BACnetObject> {
        Box::new(AnalogInputObject::new(instance, name, EngineeringUnits::NO_UNITS).unwrap())
    }

    /// Both duplicate kinds surface as ValueError, named for what clashed:
    /// each from its typed protocol error, the only form they take (#1833).
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
            let Err(error) = build_database(&identity, extra, None) else {
                panic!("{expected}: the database must be refused");
            };
            Python::attach(|py| {
                assert!(error.is_instance_of::<PyValueError>(py), "{expected}");
                assert!(error.to_string().contains(expected), "{error}");
            });
        }
    }

    /// A second start is a lifecycle refusal: `BacnetStateError` with
    /// `state` "already_started" and the text alone (#1833).
    #[test]
    fn a_second_start_raises_the_state_error() {
        Python::initialize();
        let error = owner_error(super::super::lifecycle::LifecycleError::<PyErr>::AlreadyStarted);
        Python::attach(|py| {
            let class = crate::errors::state_error_type(py).unwrap().bind(py);
            let value = error.value(py);
            assert!(value.get_type().is(class));
            assert_eq!(
                value.str().unwrap().to_str().unwrap(),
                "endpoint already started"
            );
            let state: String = value.getattr("state").unwrap().extract().unwrap();
            assert_eq!(state, "already_started");
        });
    }
}
