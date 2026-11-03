//! The B/IP endpoint's source Audit Reporter keywords (#2348).
//!
//! `EndpointSession::with_source_audit_reporter` runs on direct IPv4 B/IP
//! only, so only `BipEndpoint` takes these. They are parsed once at
//! construction and kept for the rebuild at each start, as the object
//! keywords are (#1867).

use std::net::SocketAddrV4;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use crate::errors::to_py_err;
use crate::types::PyObjectIdentifier;

/// A validated source Reporter selection: the Reporter, the Device's Audit
/// recipient and the routes to recipient Devices.
#[derive(Clone, Debug)]
pub(super) struct SourceAuditKeywords {
    reporter: ObjectIdentifier,
    recipient: BACnetRecipient,
    bindings: Vec<(ObjectIdentifier, SocketAddrV4)>,
}

impl SourceAuditKeywords {
    /// Parse `source_audit_reporter`, `audit_recipient` and
    /// `audit_device_bindings`. What construction can check is refused here
    /// with ValueError or TypeError: a Reporter identifier that names no
    /// concrete Audit Reporter, a recipient or binding without a Reporter, a
    /// Reporter without a recipient or without `device_write_policy`, this
    /// device as the recipient, and bad or duplicate bindings. Whether the
    /// Reporter is in the database is checked at start, once the `add_*`
    /// calls are in.
    pub(super) fn parse(
        reporter: Option<PyObjectIdentifier>,
        recipient: Option<&Bound<'_, PyAny>>,
        bindings: Option<Vec<(PyObjectIdentifier, String)>>,
        device_instance: u32,
        device_writes: bool,
    ) -> PyResult<Option<Self>> {
        let Some(reporter) = reporter.map(|oid| oid.to_rust()) else {
            if recipient.is_some() || bindings.is_some() {
                return Err(PyValueError::new_err(
                    "audit_recipient and audit_device_bindings need source_audit_reporter",
                ));
            }
            return Ok(None);
        };
        if reporter.object_type() != ObjectType::AUDIT_REPORTER
            || reporter.instance_number() == ObjectIdentifier::MAX_INSTANCE
        {
            return Err(PyValueError::new_err(
                "source_audit_reporter must name a concrete Audit Reporter object",
            ));
        }
        if !device_writes {
            return Err(PyValueError::new_err(
                "source_audit_reporter needs device_write_policy: the Audit recipient \
                 it reports to is writable on the local Device",
            ));
        }
        let recipient = recipient
            .ok_or_else(|| PyValueError::new_err("source_audit_reporter needs audit_recipient"))?;
        let recipient = crate::types::audit_recipient_from_py(recipient, "audit_recipient")?;
        if let BACnetRecipient::Device(device) = &recipient
            && (device.object_type() != ObjectType::DEVICE
                || device.instance_number() == ObjectIdentifier::MAX_INSTANCE
                || device.instance_number() == device_instance)
        {
            return Err(PyValueError::new_err(
                "audit_recipient must be a concrete remote Device or a B/IP address",
            ));
        }
        let mut parsed: Vec<(ObjectIdentifier, SocketAddrV4)> = Vec::new();
        for (index, (device, address)) in bindings.into_iter().flatten().enumerate() {
            let device = device.to_rust();
            if device.object_type() != ObjectType::DEVICE
                || device.instance_number() == ObjectIdentifier::MAX_INSTANCE
            {
                return Err(PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] must name a concrete Device"
                )));
            }
            let address: SocketAddrV4 = address.parse().map_err(|_| {
                PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] address must be 'ipv4:port'"
                ))
            })?;
            if parsed.iter().any(|(known, _)| *known == device) {
                return Err(PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] repeats a Device"
                )));
            }
            parsed.push((device, address));
        }
        Ok(Some(Self {
            reporter,
            recipient,
            bindings: parsed,
        }))
    }

    /// Provision the Device's Audit recipient in the database built for this
    /// start.
    pub(super) fn provision(
        &self,
        identity: &DeviceIdentity,
        db: &mut ObjectDatabase,
    ) -> PyResult<()> {
        db.get_mut(&identity.device_oid())
            .and_then(|device| device.device_authority_internal())
            .ok_or_else(|| {
                PyRuntimeError::new_err("the endpoint Device lacks its Audit authority")
            })?
            .provision_audit_recipient(self.recipient.clone())
            .map_err(to_py_err)
    }

    /// Add the Device routes to the builder.
    pub(super) fn bind(&self, builder: BipEndpointBuilder) -> BipEndpointBuilder {
        self.bindings
            .iter()
            .fold(builder, |builder, (device, address)| {
                builder.source_audit_device_binding(*device, *address)
            })
    }

    /// The Reporter the session selects. Its start refuses one that isn't an
    /// Audit Reporter in the database, with a BacnetError naming it.
    pub(super) fn reporter(&self) -> ObjectIdentifier {
        self.reporter
    }
}
