//! Complete target recipient mutation owner: preparation, commit and owned delivery.
use super::audit_reporter::{
    DeliveryCompletion, deliver, deliver_global_broadcast, encode_notification,
};
use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::{
    audit::{AuditReporterChangeSink, AuditReporterStatus, TargetAuditAssociation},
    clock::ClockFrame,
    database::{AuditOwnership, EventSequence},
    device::{AuditRecipientChangeSink, AuditWriteSource, DeviceAuthority},
};
use bacnet_types::constructed::{AuditPropertyReference, BACnetAuditNotification, BACnetRecipient};
use bacnet_types::sync::MutexExt;
use bacnet_types::{enums::AuditOperation, primitives::BACnetTimeStamp};

pub(super) struct TargetAudit<T: TransportPort> {
    pub(super) batches: Arc<super::audit_batch_queue::AuditBatchQueue>,
    pub(super) owner: Arc<AuditOwnership>,
    pub(super) device: ObjectIdentifier,
    pub(super) association: Arc<TargetAuditAssociation>,
    /// The recipient this device reports to now. Its route is resolved at
    /// each use ([`Self::current_route`]), so a local network number
    /// published after startup applies to the next notification (#1358).
    pub(super) recipient: std::sync::Mutex<BACnetRecipient>,
    pub(super) routes: Arc<super::audit_recipient_routes::AuditRoutes>,
    pub(super) sequence: Arc<EventSequence>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) transactions: Arc<NotificationTransactions>,
    pub(super) max_apdu: u32,
}

pub(super) fn denied() -> Error {
    Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
}

/// The audit authority of the identity's built-in Device `device`, which
/// [`validate`] checks and [`TargetAudit::install`] installs on.
fn device_authority(
    db: &mut ObjectDatabase,
    device: ObjectIdentifier,
) -> Result<DeviceAuthority<'_>, Error> {
    db.get_mut(&device)
        .and_then(|object| object.device_authority_internal())
        .filter(|authority| authority.object_identifier() == device)
        .ok_or_else(|| {
            Error::invalid_config("target Audit requires the identity's built-in Device")
        })
}

/// The recipient provisioned on `authority`'s Device.
fn provisioned_recipient(authority: &DeviceAuthority<'_>) -> Result<BACnetRecipient, Error> {
    authority
        .provisioned_audit_recipient()
        .cloned()
        .ok_or_else(|| {
            Error::invalid_config("target Audit requires a provisioned built-in Device recipient")
        })
}

/// The status of the Audit Reporter `selected`, which [`validate`] checks
/// and [`TargetAudit::install`] associates.
fn reporter_status(
    db: &ObjectDatabase,
    selected: &ObjectIdentifier,
) -> Result<Arc<AuditReporterStatus>, Error> {
    db.get(selected)
        .and_then(|object| object.audit_reporter())
        .filter(|object| object.object_identifier() == *selected)
        .map(|reporter| reporter.status_internal())
        .ok_or_else(|| Error::invalid_config("invalid audit reporter: missing read capability"))
}

/// Validate before transport startup and before installing any capability.
pub(super) fn validate(
    db: &mut ObjectDatabase,
    config: &ServerConfig,
    routes: &super::audit_recipient_routes::AuditRoutes,
) -> Result<(), Error> {
    let Some(profile) = &config.audit_reporters else {
        return Ok(());
    };
    db.validate_audit_installation_internal()?;
    // The identity's Device is the audit identity, as it is every
    // notification's initiator (#2217). The installed runtime refuses new
    // Devices and protects this one while it lives.
    let device = crate::local_device::initiating_device(&config.device);
    let authority = device_authority(db, device)?;
    authority.validate_audit_recipient_installation()?;
    let value = provisioned_recipient(&authority)?;
    let mut statuses = Vec::with_capacity(profile.reporters.len());
    for selected in &profile.reporters {
        statuses.push(reporter_status(db, selected)?);
        db.get_mut(selected).and_then(|object| object.audit_reporter_mut())
            .filter(|object| object.object_identifier() == *selected)
            .ok_or_else(|| Error::InvalidConfig(bacnet_types::error::Refusal::text(format!("invalid audit reporter: selected object {selected:?} is absent or lacks the Audit Reporter capability"))))?
            .validate_installation()?;
    }
    let route = routes.resolve(&value);
    // A Device may be provisioned before its route is available. Address choices
    // must belong to this runtime's explicit direct-unicast B/IP subset. One
    // naming a network that the number in force does not name, or not yet,
    // starts unresolved the same way, and resolves once it does (#1460).
    if matches!(value, BACnetRecipient::Address(_))
        && route.is_none()
        && !routes.awaits_local_number(&value)
    {
        return Err(denied());
    }
    for status in &statuses {
        super::audit_context_preparation::validate_summary(
            status,
            &status.configuration(),
            device,
            route.as_ref(),
            u32::from(config.device.max_apdu_length()),
        )?;
        // Set here, at prepare and finish, only. A local network number learned
        // later can make a routed binding at the link broadcast MAC unusable
        // (`RecipientRoute::localize`): notifications then find no route, but
        // this flag keeps its startup value. A confirmed reporter has no route
        // to a station at a broadcast or group form on another network (#2732).
        let confirmed = status.configuration().confirmed;
        status.set_configured(route.as_ref().is_some_and(|route| route.serves(confirmed)));
    }
    Ok(())
}

impl<T: TransportPort + 'static> TargetAudit<T> {
    pub(super) fn install(
        db: &mut ObjectDatabase,
        config: &ServerConfig,
        routes: Arc<super::audit_recipient_routes::AuditRoutes>,
        network: &Arc<NetworkLayer<T>>,
        transactions: &Arc<NotificationTransactions>,
    ) -> Result<Option<Arc<Self>>, Error> {
        let Some(profile) = &config.audit_reporters else {
            return Ok(None);
        };
        let device = crate::local_device::initiating_device(&config.device);
        let association = TargetAuditAssociation::new(
            profile
                .reporters
                .iter()
                .map(|oid| Ok((*oid, reporter_status(db, oid)?)))
                .collect::<Result<_, Error>>()?,
        );
        let recipient = provisioned_recipient(&device_authority(db, device)?)?;
        let runtime = Arc::new(Self {
            batches: super::audit_batch_queue::AuditBatchQueue::new(&association),
            owner: AuditOwnership::for_target(device, Arc::clone(&association)),
            device,
            association: Arc::clone(&association),
            recipient: std::sync::Mutex::new(recipient),
            routes: Arc::clone(&routes),
            sequence: db.event_sequence_internal(),
            network: Arc::clone(network),
            transactions: Arc::clone(transactions),
            max_apdu: u32::from(config.device.max_apdu_length()),
        });
        let sink: Arc<dyn AuditRecipientChangeSink> = runtime.clone();
        device_authority(db, device)?.install_audit_recipient(&sink)?;
        let reporter_sink: Arc<dyn AuditReporterChangeSink> = runtime.clone();
        for selected in &profile.reporters {
            db.get_mut(selected)
                .and_then(|object| object.audit_reporter_mut())
                .ok_or_else(denied)?
                .install(&reporter_sink, &runtime.owner)?;
        }
        transactions.install_target_audit(association);
        db.protect_audit_internal(&runtime.owner)?;
        assert!(transactions.audit_routes.set(routes).is_ok());
        transactions.set_audit_owner(&runtime.owner);
        assert!(
            transactions
                .audit_batch
                .set(Arc::downgrade(&runtime.batches))
                .is_ok()
        );
        super::audit_batch_runtime::start(
            Arc::clone(&runtime.batches),
            Arc::clone(network),
            transactions,
        );
        Ok(Some(runtime))
    }

    /// The route to the current recipient, resolved now.
    pub(super) fn current_route(
        &self,
    ) -> Option<Arc<super::event_recipient_route::ConfirmedRecipientRoute>> {
        self.routes.resolve(&self.recipient.locked())
    }

    /// The network's number changed: whether the recipient resolves may have
    /// changed with it, so each Reporter's CONFIGURATION_ERROR follows the
    /// route in force now, not at the next audited operation (#1460).
    pub(super) fn number_changed(&self) {
        let route = self.current_route();
        for (_, status) in self.association.reporters() {
            let confirmed = status.configuration().confirmed;
            status.set_configured(route.as_ref().is_some_and(|route| route.serves(confirmed)));
        }
    }

    pub(super) fn uninstall(self: &Arc<Self>, db: &mut ObjectDatabase) {
        let sink: Arc<dyn AuditRecipientChangeSink> = self.clone();
        if let Some(mut authority) = db
            .get_mut(&self.device)
            .and_then(|object| object.device_authority_internal())
        {
            authority.uninstall_audit_recipient(&sink);
        }
        let reporter_sink: Arc<dyn AuditReporterChangeSink> = self.clone();
        for (selected, _) in self.association.reporters() {
            if let Some(mut authority) = db
                .get_mut(selected)
                .and_then(|object| object.audit_reporter_mut())
            {
                authority.uninstall(&reporter_sink);
            }
        }
        db.release_audit_internal(&self.owner);
    }

    fn commit(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        source: Option<&AuditWriteSource>,
        timestamp: BACnetTimeStamp,
    ) -> Result<(), Error> {
        let selected = self.association.select_recipient_change(self.device);
        let status = selected.status;
        self.transactions.commit_audit(|| {
            if !self.owner.is_active() {
                return Err(denied());
            }
            // An old Address the network's number no longer, or not yet,
            // names has no route, but does not refuse the change (#1460,
            // #1461): the record goes by global broadcast in its place.
            // Any other old recipient must resolve.
            let old_route = match self.routes.resolve(current) {
                Some(route) => Some(route),
                None if self.routes.awaits_local_number(current) => None,
                None => return Err(denied()),
            };
            let new_route = self.routes.resolve(&new).ok_or_else(denied)?;
            let mut old_value = BytesMut::new();
            let mut new_value = BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(&mut old_value, current)?;
            bacnet_encoding::constructed::encode_recipient(&mut new_value, &new)?;
            let notification = BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: Some(timestamp),
                source_device: source.map_or(BACnetRecipient::Device(self.device), |source| {
                    source.device.clone()
                }),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: source.map(|source| source.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(self.device),
                target_object: Some(self.device),
                target_property: Some(AuditPropertyReference {
                    property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    property_array_index: None,
                }),
                target_priority: None,
                target_value: Some(new_value.to_vec()),
                current_value: Some(old_value.to_vec()),
                result: None,
            };
            let mut contexts = Vec::with_capacity(self.association.reporters().len());
            for (_, reporter) in self.association.reporters() {
                super::audit_context_preparation::validate_summary(
                    reporter,
                    &reporter.configuration(),
                    self.device,
                    Some(&new_route),
                    self.max_apdu,
                )?;
                let next = reporter.next_configuration_epoch()?;
                let context = self
                    .transactions
                    .audit_failure_queue(reporter)
                    .expect("target context owner")
                    .prepare_context(next, reporter.auditing_failure_epoch().is_some())?;
                contexts.push((Arc::clone(reporter), next, context));
            }
            let task = status.commit_recipient_change(|confirmed, token| {
                // Clause 12.11.66: the change's record reaches both
                // recipients, or goes by global broadcast. Each recipient with
                // a route gets its copy, and with no route to one of them an
                // unconfirmed global broadcast (`None`) stands in for its
                // copy. A confirmed copy has no route to a station at a
                // broadcast or group form on another network (#2732).
                let routes: Vec<_> = [old_route, Some(new_route)]
                    .into_iter()
                    .map(|route| route.filter(|route| route.serves(confirmed)))
                    .collect();
                let stand_in = routes.iter().any(Option::is_none);
                let targets: Vec<_> = routes
                    .into_iter()
                    .flatten()
                    .map(Some)
                    .chain(stand_in.then_some(None))
                    .collect();
                let mut attempts = Vec::with_capacity(targets.len());
                for target in targets {
                    let permit = self.transactions.try_admit_audit().map_err(|_| denied())?;
                    let reservation = match &target {
                        Some(route) if confirmed => Some(
                            self.transactions
                                .reserve(
                                    route.canonical_peer.clone(),
                                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                                )
                                .map_err(|_| denied())?,
                        ),
                        _ => None,
                    };
                    let invoke = reservation
                        .as_ref()
                        .map_or(0, |(operation, _)| operation.invoke_id());
                    let bytes = encode_notification(
                        &notification,
                        reservation.is_some(),
                        self.max_apdu,
                        invoke,
                    )
                    .ok_or_else(denied)?;
                    attempts.push((target, permit, reservation, bytes));
                }
                // No fallible work remains. Worker registration happens under
                // the same owner lock as this commit and shutdown sealing.
                *self.recipient.locked() = new.clone();
                *current = new;
                // `contexts` holds every associated Reporter with its next
                // epoch, so recording them looks nothing up.
                for (other, next, _) in &contexts {
                    if !Arc::ptr_eq(other, &status) {
                        other.commit_recipient_epoch(*next);
                    }
                }
                let network = Arc::clone(&self.network);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                // Each attempt owns its completion, so each outcome reaches
                // the Reporter's health on its own.
                let runs: Vec<_> = attempts
                    .into_iter()
                    .map(|(target, permit, reservation, bytes)| {
                        let network = Arc::clone(&network);
                        let completion = DeliveryCompletion {
                            status: Arc::clone(&status),
                            epoch: token,
                            finished: false,
                        };
                        async move {
                            let _permit = permit;
                            let delivered = match &target {
                                Some(route) => {
                                    deliver(&network, route, &bytes, reservation, deadline).await
                                }
                                None => deliver_global_broadcast(&network, &bytes, deadline).await,
                            };
                            completion.finish(delivered);
                        }
                    })
                    .collect();
                Ok(async move {
                    futures_util::future::join_all(runs).await;
                })
            })?;
            for (_, _, context) in contexts {
                context.publish();
            }
            Ok(task)
        })?;
        // Queue->status is an existing lock order. Wake only after status unlock.
        self.transactions.audit_recipient_changed();
        Ok(())
    }
}

impl<T: TransportPort + 'static> AuditRecipientChangeSink for TargetAudit<T> {
    fn is_active(&self) -> bool {
        self.owner.is_active()
    }
    fn change(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error> {
        match clock.filter(|frame| frame.is_valid_actual_datetime()) {
            Some(frame) => self.commit(
                current,
                new,
                source,
                BACnetTimeStamp::DateTime {
                    date: frame.local_date,
                    time: frame.local_time,
                },
            ),
            None => self.sequence.transaction(|number| {
                self.commit(
                    current,
                    new,
                    source,
                    BACnetTimeStamp::SequenceNumber(number),
                )
            }),
        }
    }
}

impl<T: TransportPort> TargetAudit<T> {
    pub(super) fn seal(&self) {
        self.transactions.seal_audit_owner(&self.owner);
    }
}

/// Spawn the long-lived task `make` creates, holding `owner` until it ends.
///
/// The task's future is created and boxed here rather than in the caller's
/// poll frame (#953).
pub(super) fn spawn_owned<F>(
    owner: Option<Arc<bacnet_objects::database::AuditOwnership>>,
    make: impl FnOnce() -> F,
) -> JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let future = make();
    super::heap_futures::spawn_boxed(move || async move {
        let _owner = owner;
        future.await;
    })
}

#[cfg(test)]
#[path = "audit_recipient_install_tests.rs"]
mod install_tests;
