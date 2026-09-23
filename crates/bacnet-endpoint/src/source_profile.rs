//! Complete source profile preflight, before mutation or ingress ownership.
use super::*;
impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Write the active source profile's Device Audit recipient as a trusted local operation.
    ///
    /// `Some(value)` validates and changes the actual Device value, atomically
    /// admitting its old/new notifications. `None` is NULL relinquishment: it
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

    pub(super) fn prepare_source_audit_reporter(
        &mut self,
    ) -> Result<Option<crate::source_read::recipient::SourceRoutes>, Error> {
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
        let broadcast = self
            .ingress
            .as_ref()
            .and_then(|ingress| ingress.bip_broadcast_endpoint())
            .ok_or_else(|| Error::Encoding("source Audit requires IPv4 B/IP".into()))?;
        let routes = crate::source_read::recipient::SourceRoutes::new(
            &self.source_audit_bindings,
            broadcast,
        )?;
        let db = self.database.as_mut().ok_or_else(|| {
            Error::Encoding("source Audit Reporter requires an attached local database".into())
        })?;
        // Ready sessions have not shared the Arc with a responder. Synchronous,
        // exclusive validation has no await/cancellation or concurrent mutation.
        let db = Arc::get_mut(db)
            .expect("database is unshared before startup")
            .get_mut();
        let devices = db.find_by_type(ObjectType::DEVICE);
        if devices.len() != 1 || devices[0].instance_number() == ObjectIdentifier::MAX_INSTANCE {
            return Err(Error::Encoding(
                "source Audit Reporter requires exactly one local Device".into(),
            ));
        }
        if self
            .identity
            .as_ref()
            .is_some_and(|identity| devices[0].instance_number() != identity.instance())
        {
            return Err(Error::Encoding(
                "source Audit Reporter local Device does not match session identity".into(),
            ));
        }
        let recipient = db
            .get_mut(&devices[0])
            .and_then(|object| object.device_authority_internal())
            .filter(|authority| authority.object_identifier() == devices[0])
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
            .audit_reporter_internal()
            .is_some_and(|reporter| reporter.object_identifier() == selected)
        {
            return Err(Error::Encoding(
                "selected object lacks the Audit Reporter capability".into(),
            ));
        }
        if object.audit_reporter_internal().is_some_and(|reporter| {
            reporter
                .property_list()
                .contains(&PropertyIdentifier::MONITORED_OBJECTS)
        }) {
            return Err(Error::Encoding(
                "source READ does not support Monitored_Objects".into(),
            ));
        }
        for (oid, object) in db.iter_objects() {
            if oid != selected && oid.object_type() == ObjectType::AUDIT_REPORTER {
                match object.read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None) {
                    Ok(PropertyValue::Boolean(false)) => {}
                    Ok(PropertyValue::Boolean(true)) => {
                        return Err(Error::Encoding(
                            "database already contains a conflicting source Audit Reporter".into(),
                        ))
                    }
                    _ => {
                        return Err(Error::Encoding(
                            "cannot determine another Audit Reporter's source ownership".into(),
                        ))
                    }
                }
            }
        }
        Ok(Some(routes))
    }
}
