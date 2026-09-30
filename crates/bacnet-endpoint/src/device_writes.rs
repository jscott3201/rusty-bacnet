//! Pre-start execution profile and authority for the endpoint Device responder.

use super::*;
use bacnet_server::mutation::MutationAuthorizer;
use bacnet_types::enums::{ReinitializedState, ServiceSupported};

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Enables authorized writes to the local Device's `Description` and installed Audit recipient.
    ///
    /// The callback is mandatory and receives the existing redacted mutation
    /// context. A refusal or panic denies the request before mutation. It must
    /// be fast, nonblocking and side-effect-free; see [`MutationAuthorizer`].
    /// Other objects/properties and WritePropertyMultiple remain unsupported.
    /// An authorized NULL relinquishment succeeds without changing Description.
    /// Recipient writes require the complete source runtime installed by source selection.
    ///
    /// Startup requires a server role and a concrete built-in local Device in
    /// the attached database ([`ObjectDatabase::local_device`]: the lowest
    /// instance when it holds several). Writes reach only that Device; any
    /// other Device is out of scope. A composed identity must match it and
    /// contain only the services the responder executes. Validation errors
    /// precede configuration mutation and transport startup, allowing correction
    /// and retry. The Device and identity then advertise exactly those services;
    /// the default session remains ReadProperty-only.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_device_writes(mut self, authorizer: MutationAuthorizer) -> Self {
        self.assert_configurable();
        self.device_write_authorizer = Some(authorizer);
        self
    }

    /// Enables ReinitializeDevice, carried out by `handler` with the database write-locked.
    ///
    /// Startup requirements and the advertised services are those of
    /// [`with_device_writes`](Self::with_device_writes). Without a password set by
    /// [`with_reinit_password`](Self::with_reinit_password), any peer's request reaches `handler`.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(ReinitializedState, &mut ObjectDatabase) -> Result<(), Error> + Send + Sync + 'static,
    {
        self.assert_configurable();
        self.reinitialize = Some(Arc::new(handler));
        self
    }

    /// Sets the password a ReinitializeDevice request must carry.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_reinit_password(mut self, password: impl Into<String>) -> Self {
        self.assert_configurable();
        self.reinit_password = Some(password.into());
        self
    }

    /// The services the responder executes with the capabilities enabled on this session.
    fn executed_services(&self) -> Vec<ServiceSupported> {
        let mut services = vec![ServiceSupported::READ_PROPERTY];
        if self.device_write_authorizer.is_some() {
            services.push(ServiceSupported::WRITE_PROPERTY);
        }
        if self.reinitialize.is_some() {
            services.push(ServiceSupported::REINITIALIZE_DEVICE);
        }
        services
    }

    pub(super) fn validate_device_execution(&mut self) -> Result<Option<ObjectIdentifier>, Error> {
        let writes = self.device_write_authorizer.is_some();
        let reinitialize = self.reinitialize.is_some();
        // Names the capability that needs a Device, keeping the Device-writes messages as they were.
        let (capability, subject) = if writes {
            ("Device writes", "Device writes require")
        } else {
            ("ReinitializeDevice", "ReinitializeDevice requires")
        };
        if self.role == SessionRole::ClientOnly {
            return if writes || reinitialize {
                Err(Error::Encoding(format!("{subject} a server role")))
            } else {
                Ok(None)
            };
        }
        let allowed = self.executed_services();
        if self.identity.as_ref().is_some_and(|identity| {
            !identity
                .services()
                .contains(&ServiceSupported::READ_PROPERTY)
                || identity
                    .services()
                    .iter()
                    .any(|service| !allowed.contains(service))
        }) {
            return Err(Error::Encoding(
                "Endpoint identity services do not match the responder's services".into(),
            ));
        }
        if !writes && !reinitialize {
            return Ok(None);
        }
        let db = self
            .database
            .as_mut()
            .ok_or_else(|| Error::Encoding(format!("{subject} an attached local database")))?;
        let db = Arc::get_mut(db)
            .expect("database is unshared before startup")
            .get_mut();
        // The target is the selected Device, the one wildcard reads resolve
        // to, and it must be concrete.
        let oid = db
            .local_device()
            .identifier()
            .ok_or_else(|| Error::Encoding(format!("{subject} a concrete local Device")))?;
        if self
            .identity
            .as_ref()
            .is_some_and(|identity| identity.device_oid() != oid)
        {
            return Err(Error::Encoding(format!(
                "{capability} local Device does not match session identity"
            )));
        }
        if !db
            .get_mut(&oid)
            .expect("Device exists")
            .device_authority_internal()
            .is_some_and(|device| device.object_identifier() == oid)
        {
            return Err(Error::Encoding(format!(
                "{subject} the built-in Device authority"
            )));
        }
        Ok(Some(oid))
    }

    // All fallible configuration validation (including source ownership) precedes
    // this commit. No await/callback or second copy of Device state is involved.
    pub(super) fn commit_device_write_profile(&mut self, target: Option<ObjectIdentifier>) {
        let Some(oid) = target else { return };
        let services = self.executed_services();
        let db = Arc::get_mut(self.database.as_mut().expect("validated database"))
            .expect("database is unshared before startup")
            .get_mut();
        db.get_mut(&oid)
            .expect("validated Device")
            .device_authority_internal()
            .expect("validated Device authority")
            .set_services_supported(&services);
        if let Some(identity) = self.identity.take() {
            self.identity = Some(identity.with_services(&services));
        }
    }
}
