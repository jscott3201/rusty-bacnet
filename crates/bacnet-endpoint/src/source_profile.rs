//! Complete source profile preflight, before mutation or ingress ownership.
use super::*;
use bacnet_objects::property_metadata::has_property;
/// Check a builder's link bindings against the node's own `local` MAC
/// before any I/O, as [`EndpointSession::start`] will (#2357).
pub(crate) fn check_link_bindings(
    bindings: &[(ObjectIdentifier, bacnet_types::MacAddr)],
    local: &[u8],
) -> Result<(), Error> {
    crate::source_audit::recipient::SourceRoutes::for_link(
        bindings,
        crate::source_audit::recipient::SourceLink::Other {
            local: bacnet_types::MacAddr::from_slice(local),
            groups: bacnet_transport::port::GroupDestinations::unknown(),
        },
    )
    .map(drop)
}

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Write the active source profile's Device Audit recipient as a trusted local operation.
    ///
    /// `Some(value)` validates and changes the actual Device value, atomically
    /// admitting its two notifications: to the old and the new recipient, or
    /// to the new one and by global broadcast when the old is an Address the
    /// session's number does not name. `None` is NULL relinquishment: it
    /// succeeds unchanged after the same live-owner checks. Equal values are also
    /// no-ops. This is runtime mutation, not initial provisioning or route setup.
    ///
    /// Available in a running `ClientOnly` or `Both` source session. Local calls
    /// identify the local Device and have no network invoke ID; they do not invoke
    /// the inbound network authorizer. Unavailable routes or precommit resource
    /// failures leave state unchanged. Success establishes commit and owned
    /// delivery admission, not remote receipt. The session's sealed state is
    /// rechecked after acquiring the database; pre-start, stopping, stopped and
    /// sessions without a source owner reject the operation.
    pub async fn write_audit_recipient(
        &self,
        recipient: Option<bacnet_types::constructed::BACnetRecipient>,
    ) -> Result<(), Error> {
        let unavailable = || Error::Encoding("endpoint source Audit owner is not running".into());
        if !self.is_running() {
            return Err(unavailable());
        }
        let database = self.database.as_ref().ok_or_else(unavailable)?;
        let mut db = database.write().await;
        let runtime = self
            .source_recipient
            .as_ref()
            .filter(|runtime| self.is_running() && runtime.owner.is_active())
            .ok_or_else(unavailable)?;
        runtime.write_local(&mut db, recipient)
    }

    /// Bind a Device to `mac` on this session's link, for source Audit
    /// delivery to that Device (#2357), on any link but IPv4 B/IP, whose
    /// bindings are [`BipEndpointBuilder::source_audit_device_binding`]'s.
    /// `mac` is in the link's format: a 3-octet VMAC on B/IPv6, a 6-octet
    /// VMAC on BACnet/SC, a 1-octet station on MS/TP. It is route data only:
    /// provision the recipient on the Device itself.
    ///
    /// [`start`](Self::start) refuses, with SERVICES /
    /// SERVICE_REQUEST_DENIED, a MAC that isn't a direct unicast destination
    /// on the link: of another length than the node's own, the node's own,
    /// a group destination (MS/TP's station 255, SC's broadcast VMAC), or a
    /// six-octet all-zero MAC (SC's reserved VMAC), and refuses a non-Device
    /// or wildcard identifier or a repeated Device with an error naming it.
    /// A link that learns its MAC at start (B/IPv6) checks the node's own
    /// only then: a binding to it starts, and the Reporter reads
    /// CONFIGURATION_ERROR. These bindings on a B/IP session, or B/IP ones
    /// on another link, fail start. Without a selected source Reporter they
    /// are inert.
    ///
    /// [`BipEndpointBuilder::source_audit_device_binding`]: crate::bip::BipEndpointBuilder::source_audit_device_binding
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_source_audit_device_binding(
        mut self,
        device: ObjectIdentifier,
        mac: bacnet_types::MacAddr,
    ) -> Self {
        self.assert_configurable();
        self.source_audit_link_bindings.push((device, mac));
        self
    }

    /// Hand a builder's link bindings to the session.
    pub(crate) fn with_source_audit_device_bindings(
        mut self,
        bindings: Vec<(ObjectIdentifier, bacnet_types::MacAddr)>,
    ) -> Self {
        for (device, mac) in bindings {
            self = self.with_source_audit_device_binding(device, mac);
        }
        self
    }

    /// The routes the selected source Reporter delivers by, checked against
    /// the link the session will start: IPv4 B/IP's broadcast endpoint, or
    /// any other link's own MAC and group rule (#2357).
    fn prepare_source_routes(&self) -> Result<crate::source_audit::recipient::SourceRoutes, Error> {
        use crate::source_audit::recipient::{SourceLink, SourceRoutes};
        let ingress = self
            .ingress
            .as_ref()
            .ok_or_else(|| Error::Encoding("source Audit requires the session's link".into()))?;
        match ingress.bip_broadcast_endpoint() {
            Some(broadcast) => {
                if !self.source_audit_link_bindings.is_empty() {
                    return Err(Error::Encoding(
                        "source Audit link bindings need a link other than IPv4 B/IP; \
                         B/IP takes source_audit_device_binding"
                            .into(),
                    ));
                }
                SourceRoutes::new(&self.source_audit_bindings, broadcast)
            }
            None => {
                if !self.source_audit_bindings.is_empty() {
                    return Err(Error::Encoding(
                        "B/IP source Audit bindings need an IPv4 B/IP link".into(),
                    ));
                }
                let (local, groups) = ingress.link_unicast_facts().ok_or_else(|| {
                    Error::Encoding("source Audit requires the session's link".into())
                })?;
                SourceRoutes::for_link(
                    &self.source_audit_link_bindings,
                    SourceLink::Other { local, groups },
                )
            }
        }
    }

    pub(super) fn prepare_source_audit_reporter(
        &mut self,
    ) -> Result<Option<crate::source_audit::recipient::SourceRoutes>, Error> {
        let Some(selected) = self.source_audit_reporter else {
            return Ok(None);
        };
        if self.role == SessionRole::ServerOnly {
            return Err(Error::Encoding(
                "source Audit Reporter requires a client role".into(),
            ));
        }
        if self.role == SessionRole::Both && self.device_write_authorizer.is_none() {
            return Err(Error::Encoding(
                "source Audit with Both roles requires an explicit Device write authorizer".into(),
            ));
        }
        let routes = self.prepare_source_routes()?;
        let db = self.database.as_mut().ok_or_else(|| {
            Error::Encoding("source Audit Reporter requires an attached local database".into())
        })?;
        // Ready sessions have not shared the Arc with a responder. Synchronous,
        // exclusive validation has no await/cancellation or concurrent mutation.
        let db = Arc::get_mut(db)
            .expect("database is unshared before startup")
            .get_mut();
        // The local Device is the audit identity. Source ownership refuses any
        // new Device and keeps this one, so it stays the lowest while owned.
        let device = db.local_device().identifier().ok_or_else(|| {
            Error::Encoding("source Audit Reporter requires a concrete local Device".into())
        })?;
        if self
            .identity
            .as_ref()
            .is_some_and(|identity| device.instance_number() != identity.instance())
        {
            return Err(Error::Encoding(
                "source Audit Reporter local Device does not match session identity".into(),
            ));
        }
        let recipient = db
            .get_mut(&device)
            .and_then(|object| object.device_authority_internal())
            .filter(|authority| authority.object_identifier() == device)
            .and_then(|authority| authority.provisioned_audit_recipient().cloned())
            .ok_or_else(|| {
                Error::Encoding(
                    "source Audit requires a provisioned built-in Device recipient".into(),
                )
            })?;
        routes.validate_initial(&recipient)?;
        if selected.object_type() != ObjectType::AUDIT_REPORTER {
            return Err(Error::Encoding(
                "source selection must be an Audit Reporter".into(),
            ));
        }
        let object = db.get(&selected).ok_or_else(|| {
            Error::Encoding("selected source Audit Reporter is absent from the database".into())
        })?;
        if !object
            .audit_reporter()
            .is_some_and(|reporter| reporter.object_identifier() == selected)
        {
            return Err(Error::Encoding(
                "selected object lacks the Audit Reporter capability".into(),
            ));
        }
        if object.audit_reporter().is_some_and(|reporter| {
            has_property(
                &reporter.property_metadata(),
                PropertyIdentifier::MONITORED_OBJECTS,
            ) || reporter
                .configuration_internal()
                .maximum_send_delay
                .is_some()
        }) {
            return Err(Error::Encoding(
                "source Audit does not support Monitored_Objects or delayed notifications".into(),
            ));
        }
        for (oid, object) in db.iter_objects() {
            if oid != selected && oid.object_type() == ObjectType::AUDIT_REPORTER {
                match object.read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None) {
                    Ok(PropertyValue::Boolean(false)) => {}
                    Ok(PropertyValue::Boolean(true)) => {
                        return Err(Error::Encoding(
                            "database already contains a conflicting source Audit Reporter".into(),
                        ));
                    }
                    _ => {
                        return Err(Error::Encoding(
                            "cannot determine another Audit Reporter's source ownership".into(),
                        ));
                    }
                }
            }
        }
        Ok(Some(routes))
    }
}
