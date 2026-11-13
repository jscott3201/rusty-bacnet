//! Pre-start execution profile and authority for the endpoint Device responder.

use super::*;
use bacnet_server::mutation::DeviceWriteAuthorizer;
use bacnet_server::server::ReinitializeContext;
use bacnet_types::enums::{AuditOperation, ServiceSupported};

/// The services the session's server role, its bounded Device responder,
/// executes on every session, as `BACnetServicesSupported` bits (Clause 21).
///
/// The responder is not the full `bacnet-server` service set: everything not
/// here or in [`RESPONDER_OPT_IN_SERVICES`] is refused.
pub const RESPONDER_SERVICES: &[ServiceSupported] = &[ServiceSupported::READ_PROPERTY];

/// A service the session's server role executes only once an
/// [`EndpointSession`] method enables it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponderOptIn {
    /// The service, as its `BACnetServicesSupported` bit.
    pub service: ServiceSupported,
    /// The name of the [`EndpointSession`] method that enables it.
    pub method: &'static str,
}

/// The services the server role executes only once enabled, with the method
/// that enables each. The local Device and the session identity then
/// advertise them too.
pub const RESPONDER_OPT_IN_SERVICES: &[ResponderOptIn] = &[
    ResponderOptIn {
        service: ServiceSupported::WRITE_PROPERTY,
        method: "with_device_writes",
    },
    ResponderOptIn {
        service: ServiceSupported::REINITIALIZE_DEVICE,
        method: "with_reinitialize",
    },
    ResponderOptIn {
        service: ServiceSupported::SUBSCRIBE_COV,
        method: "with_cov",
    },
    ResponderOptIn {
        service: ServiceSupported::SUBSCRIBE_COV_PROPERTY,
        method: "with_cov",
    },
    ResponderOptIn {
        service: ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
        method: "with_cov",
    },
];

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Enables authorized writes to the local Device's `Description` and installed Audit recipient.
    ///
    /// The callback is mandatory and receives the existing redacted mutation
    /// context. A refusal or panic denies the request before mutation. It must
    /// be fast, nonblocking and side-effect-free; see [`DeviceWriteAuthorizer`].
    /// Other objects/properties and WritePropertyMultiple remain unsupported.
    /// An authorized NULL relinquishment succeeds without changing Description.
    ///
    /// A Device built with `DeviceObject::with_tags_persistence` keeps a
    /// written Description in that storage (#2364): the save runs before the
    /// write is answered, with the database guard dropped, and a failed one
    /// refuses the write with DEVICE / OPERATIONAL_PROBLEM. The session's
    /// one dispatch task waits for it, so until the save returns the session
    /// handles no other request and no reply to its client role, and
    /// [`stop`](Self::stop) waits as well. Storage that never returns holds
    /// the whole session: use storage that answers quickly or fails.
    /// Recipient writes require the complete source runtime installed by source selection.
    ///
    /// Startup requires a server role and a concrete built-in local Device in
    /// the attached database ([`ObjectDatabase::local_device`]: the lowest
    /// instance when it holds several, which preflight requires to be the
    /// identity's). Writes reach only that Device; any
    /// other Device is out of scope. A composed identity must match it and
    /// contain only the services the responder executes. Validation errors
    /// precede configuration mutation and transport startup, allowing correction
    /// and retry. The Device and identity then advertise exactly those services;
    /// the default session remains ReadProperty-only.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_device_writes(mut self, authorizer: DeviceWriteAuthorizer) -> Self {
        self.assert_configurable();
        self.device_write_authorizer = Some(authorizer);
        self
    }

    /// Enables ReinitializeDevice, carried out by `handler` with the
    /// requester's [`ReinitializeContext`] and the database write-locked.
    ///
    /// Startup requirements and the advertised services are those of
    /// [`with_device_writes`](Self::with_device_writes). The handler follows
    /// the rules on
    /// [`ReinitializeHandler`](bacnet_server::server::ReinitializeHandler).
    /// The SimpleACK goes out only after it returns, so register any restart
    /// with [`ReinitializeContext::after_reply`] instead of restarting
    /// inline: the hook runs once the egress has sent the SimpleACK, on the
    /// same dispatch task, and never for a refusal. It runs synchronously
    /// on the session's single dispatch task, so a slow or blocking handler
    /// stalls the whole session: hand slow work such as backup files to a
    /// task. Without a password set by
    /// [`with_reinit_password`](Self::with_reinit_password) any peer reaches
    /// it, and the Device-write authorizer doesn't cover this service, so
    /// restrict sources through the context. Refuse with [`Error::protocol`];
    /// a panic or an [`Error::reject`] is answered SERVICES / OTHER and the
    /// session keeps serving.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.assert_configurable();
        self.reinitialize = Some(Arc::new(handler));
        self
    }

    /// Enables SubscribeCOV, SubscribeCOVProperty and
    /// SubscribeCOVPropertyMultiple for the objects of the session's
    /// database, and the COV notifications their subscriptions call for
    /// (#2490), under `config`. The B/IP builder sets it as
    /// [`BipEndpointBuilder::cov`](crate::bip::BipEndpointBuilder::cov).
    ///
    /// The session runs bacnet-device-core's
    /// [`Reporter`](bacnet_device_core::report::Reporter) over its database:
    /// a subscription to an object or to one property of it (with the
    /// request's own COV increment when it gives one), its renewal and its
    /// cancellation, and the lifetime running out on the session's clock.
    /// A subscriber hears first of the object's values, or the property's,
    /// when it subscribes, after the reply, then of each change the
    /// application makes through
    /// [`write_local`](Self::write_local),
    /// [`set_present_value_local`](Self::set_present_value_local) and
    /// [`set_reliability_local`](Self::set_reliability_local). A change
    /// made to the database by other means reports nothing. A confirmed
    /// notification leases its invoke ID from the pool the client role
    /// uses, is sent again on the timeout and retries of
    /// `config.confirmed`, and a subscriber that refuses it or never
    /// answers is held off for one retry cycle. The Device lists the
    /// subscriptions in Active_COV_Subscriptions and
    /// Active_COV_Multiple_Subscriptions.
    ///
    /// A SubscribeCOVPropertyMultiple subscription is one context: the
    /// subscriber's process, its references (each a property with its own
    /// COV increment and Timestamped flag), a lifetime and a
    /// Max_Notification_Delay. Its notifications are the COV-multiple ones,
    /// split to fit the Max-APDU-Length-Accepted the request stated, and
    /// what a context could not send yet goes out by its
    /// Max_Notification_Delay at the latest. A timestamped reference needs
    /// a clock the database reads
    /// ([`ObjectDatabase::set_clock_reader`]): the session installs none,
    /// so without one the reference is refused with SERVICES /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, and with one its changes carry
    /// their time of change.
    ///
    /// `config` bounds the work: `config.cov` caps the subscriptions (in
    /// all, per subscriber, indefinite ones and confirmed notifications in
    /// flight per subscriber) and what one change may send, and
    /// `config.outbox` the notifications queued at once, in entries and
    /// octets, allocated at start. A notification with no room waits, and
    /// goes with the subscription's current values once room frees; one
    /// larger than `config.outbox.octets` is never queued, so size them for
    /// the largest notification the database's objects call for (the core
    /// gives no figure to check at start). At most
    /// [`COV_CONFIRMED_IN_FLIGHT`] (64) confirmed notifications are out at
    /// once, all subscribers together, so the client role keeps three
    /// quarters of the 256 invoke IDs however many subscribers stop
    /// answering. Four silent subscribers fill it at the default
    /// `config.cov.max_confirmed_in_flight_per_peer` of 16; a confirmed
    /// report past it waits, oldest owed first, until one of them ends,
    /// which for a silent subscriber is when its retries run out. Lower that
    /// limit when many confirmed subscribers are expected. Unconfirmed
    /// notifications don't count against it. [`stop`](Self::stop) drops what
    /// is still queued and gives back every invoke ID a notification holds.
    ///
    /// A subscriber ends with the current value even when a change takes
    /// the object back to what it last heard before the last change's
    /// notification went (#2508): an unconfirmed notification still queued
    /// gives way to the newer one, and a confirmed one's Ack brings a fresh
    /// look.
    ///
    /// The Device itself takes no subscription of any kind, as on
    /// `BACnetServer`: it lends no COV reporting, so a client's write to its
    /// Description or Audit_Notification_Recipient
    /// ([`with_device_writes`](Self::with_device_writes)) notifies nobody.
    /// Startup requirements and the advertised services are those of
    /// [`with_device_writes`](Self::with_device_writes). Without a composed
    /// identity, the Device object supplies what the notifications carry of
    /// it: its identifier, as the initiating device, and for a confirmed
    /// one its Max_APDU_Length_Accepted. An invalid `config.cov`, a
    /// `config.outbox` with no entry or no octet, or a `config` that turns
    /// on the `Reporter`'s event path (a sweep) or its fault pass fails
    /// startup before the transport starts: the session hosts COV only, and
    /// sends no event notification.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_cov(mut self, config: bacnet_device_core::report::ReportConfig) -> Self {
        self.assert_configurable();
        self.cov.config = Some(config);
        self
    }

    /// Sets the password a ReinitializeDevice request must carry.
    ///
    /// It takes effect only with [`with_reinitialize`](Self::with_reinitialize):
    /// startup refuses a password without a handler.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_reinit_password(mut self, password: impl Into<String>) -> Self {
        self.assert_configurable();
        self.reinit_password = Some(password.into());
        self
    }

    /// Reports a reset the application carries out for its own reasons, not
    /// at a peer's ReinitializeDevice request: a DEVICE_RESET Audit record
    /// from this device, with no invoke ID (Clause 19.6, Table 19-5 covers
    /// resets for any reason, #1837). The counterpart of
    /// `BACnetServer::report_device_reset_local`.
    ///
    /// An accepted WARMSTART or COLDSTART is recorded already; don't report
    /// it again. The record goes through the session's source Reporter
    /// ([`with_source_audit_reporter`](Self::with_source_audit_reporter)),
    /// filtered by its Audit_Level and Auditable_Operations as a peer's
    /// reset's is, and nothing is recorded without one. It returns once the
    /// record is admitted, not delivered, so call it before the restart
    /// begins and give delivery time before the process exits.
    ///
    /// # Errors
    /// [`Error::Encoding`] when the session is not running.
    pub async fn report_device_reset_local(&self) -> Result<(), Error> {
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let (Some(source), Some(database)) = (&self.source_audit, &self.database) else {
            return Ok(());
        };
        // Synchronous under the guard: no await while it is held.
        let mut db = database.write().await;
        source.device_operation(&mut db, AuditOperation::DEVICE_RESET, None);
        Ok(())
    }

    /// The services the responder executes with the capabilities enabled on
    /// this session: [`RESPONDER_SERVICES`], then each enabled entry of
    /// [`RESPONDER_OPT_IN_SERVICES`].
    pub(crate) fn executed_services(&self) -> Vec<ServiceSupported> {
        let cov = |service| {
            matches!(
                service,
                ServiceSupported::SUBSCRIBE_COV
                    | ServiceSupported::SUBSCRIBE_COV_PROPERTY
                    | ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE
            )
        };
        let enabled = |service: ServiceSupported| {
            (service == ServiceSupported::WRITE_PROPERTY && self.device_write_authorizer.is_some())
                || (service == ServiceSupported::REINITIALIZE_DEVICE && self.reinitialize.is_some())
                || (cov(service) && self.cov.config.is_some())
        };
        let opt_in = RESPONDER_OPT_IN_SERVICES
            .iter()
            .map(|opt_in| opt_in.service)
            .filter(|service| enabled(*service));
        RESPONDER_SERVICES.iter().copied().chain(opt_in).collect()
    }

    pub(super) async fn validate_segmentation(&self) -> Result<(), Error> {
        if self.identity.as_ref().is_some_and(|identity| {
            identity.segmentation() != bacnet_types::enums::Segmentation::NO_SEGMENTATION
        }) {
            return Err(Error::Encoding(
                "EndpointSession roles require segmentation NO_SEGMENTATION".into(),
            ));
        }
        if let Some(database) = self.database.as_ref() {
            let database = database.read().await;
            if let Some(oid) = database.selected_device() {
                let device = database
                    .get(&oid)
                    .expect("selected under same database guard");
                let mode =
                    device.read_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, None)?;
                if mode
                    != PropertyValue::Enumerated(u32::from(
                        bacnet_types::enums::Segmentation::NO_SEGMENTATION.to_raw(),
                    ))
                {
                    return Err(Error::Encoding(
                        "EndpointSession selected Device requires segmentation NO_SEGMENTATION"
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_device_execution(&mut self) -> Result<Option<ObjectIdentifier>, Error> {
        let writes = self.device_write_authorizer.is_some();
        let reinitialize = self.reinitialize.is_some();
        if self.reinit_password.is_some() && !reinitialize {
            return Err(Error::Encoding(
                "a ReinitializeDevice password requires a ReinitializeDevice handler".into(),
            ));
        }
        // An empty password could never match: a request's empty password
        // decodes as none (#2094).
        if self.reinit_password.as_deref() == Some("") {
            return Err(Error::Encoding(
                "reinit_password is empty; leave it unset for no password".into(),
            ));
        }
        // Past the 20-character limit no request could carry it (#2060).
        if self
            .reinit_password
            .as_deref()
            .is_some_and(|password| !bacnet_services::device_mgmt::password_fits(password))
        {
            return Err(Error::Encoding(format!(
                "reinit_password exceeds the {}-character limit",
                bacnet_services::device_mgmt::MAX_PASSWORD_CHARS
            )));
        }
        // Names the enabled capabilities that need the Device.
        let enabled: Vec<_> = [
            (writes, "Device writes"),
            (reinitialize, "ReinitializeDevice"),
            // The three subscribe services, enabled together.
            (self.cov.config.is_some(), "COV"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        let capability = enabled.join(" and ");
        let subject = match enabled[..] {
            ["ReinitializeDevice"] | ["COV"] => format!("{capability} requires"),
            _ => format!("{capability} require"),
        };
        if self.role == SessionRole::ClientOnly {
            return if !enabled.is_empty() {
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
        if enabled.is_empty() {
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
        self.cov.prepare(self.identity.as_ref(), db, oid)?;
        Ok(Some(oid))
    }

    // All fallible configuration validation (including source ownership) precedes
    // this commit. No await/callback or second copy of Device state is involved.
    pub(super) fn commit_device_profile(&mut self, target: Option<ObjectIdentifier>) {
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
