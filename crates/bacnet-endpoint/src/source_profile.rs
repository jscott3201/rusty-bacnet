//! Complete source profile preflight, before mutation or ingress ownership.
use super::*;
impl<T: TransportPort + 'static> EndpointSession<T> {
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
