//! Pre-start authority and service profile for the narrow Device write owner.

use super::*;
use bacnet_server::mutation::MutationAuthorizer;
use bacnet_types::enums::ServiceSupported;

const SERVICES: &[ServiceSupported] = &[
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::WRITE_PROPERTY,
];

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
    /// Startup requires a server role and exactly one concrete built-in Device
    /// in the attached database. A composed identity must match that Device and
    /// contain only ReadProperty/WriteProperty service bits. Validation errors
    /// precede configuration mutation and transport startup, allowing correction
    /// and retry. The enabled Device and identity advertise exactly those two
    /// services; the default session remains ReadProperty-only.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_device_writes(mut self, authorizer: MutationAuthorizer) -> Self {
        self.assert_configurable();
        self.device_write_authorizer = Some(authorizer);
        self
    }

    pub(super) fn validate_device_writes(&mut self) -> Result<Option<ObjectIdentifier>, Error> {
        if self.device_write_authorizer.is_none() {
            return Ok(None);
        }
        if self.role == SessionRole::ClientOnly {
            return Err(Error::Encoding(
                "Device writes require a server role".into(),
            ));
        }
        if self.identity.as_ref().is_some_and(|identity| {
            identity
                .services()
                .iter()
                .any(|service| !SERVICES.contains(service))
        }) {
            return Err(Error::Encoding(
                "Device writes identity advertises unsupported services".into(),
            ));
        }
        let db = self.database.as_mut().ok_or_else(|| {
            Error::Encoding("Device writes require an attached local database".into())
        })?;
        let db = Arc::get_mut(db)
            .expect("database is unshared before startup")
            .get_mut();
        let devices = db.find_by_type(ObjectType::DEVICE);
        if devices.len() != 1 || devices[0].instance_number() == ObjectIdentifier::MAX_INSTANCE {
            return Err(Error::Encoding(
                "Device writes require exactly one concrete local Device".into(),
            ));
        }
        let oid = devices[0];
        if self
            .identity
            .as_ref()
            .is_some_and(|identity| identity.device_oid() != oid)
        {
            return Err(Error::Encoding(
                "Device writes local Device does not match session identity".into(),
            ));
        }
        if !db
            .get_mut(&oid)
            .expect("Device exists")
            .device_authority_internal()
            .is_some_and(|device| device.object_identifier() == oid)
        {
            return Err(Error::Encoding(
                "Device writes require the built-in Device authority".into(),
            ));
        }
        Ok(Some(oid))
    }

    // All fallible configuration validation (including source ownership) precedes
    // this commit. No await/callback or second copy of Device state is involved.
    pub(super) fn commit_device_write_profile(&mut self, target: Option<ObjectIdentifier>) {
        let Some(oid) = target else { return };
        let db = Arc::get_mut(self.database.as_mut().expect("validated database"))
            .expect("database is unshared before startup")
            .get_mut();
        db.get_mut(&oid)
            .expect("validated Device")
            .device_authority_internal()
            .expect("validated Device authority")
            .set_services_supported(SERVICES);
        if let Some(identity) = self.identity.take() {
            self.identity = Some(identity.with_services(SERVICES));
        }
    }
}
