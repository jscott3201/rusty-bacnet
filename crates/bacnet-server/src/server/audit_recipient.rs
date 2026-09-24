//! Complete target recipient mutation owner: preparation, commit and owned delivery.
use super::audit_reporter::{deliver, encode_notification, DeliveryCompletion};
use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::{
    audit::{AuditReporterChangeSink, TargetAuditAssociation},
    clock::ClockFrame,
    database::{AuditOwnership, EventSequence},
    device::{AuditRecipientChangeSink, AuditWriteSource},
};
use bacnet_types::constructed::{AuditPropertyReference, BACnetAuditNotification, BACnetRecipient};
use bacnet_types::{enums::AuditOperation, primitives::BACnetTimeStamp};

pub(super) struct TargetAudit<T: TransportPort> {
    pub(super) owner: Arc<AuditOwnership>,
    pub(super) device: ObjectIdentifier,
    pub(super) association: Arc<TargetAuditAssociation>,
    pub(super) current_route:
        std::sync::Mutex<Option<Arc<super::event_recipient_route::ConfirmedRecipientRoute>>>,
    pub(super) routes: Arc<super::audit_recipient_routes::AuditRoutes>,
    pub(super) sequence: Arc<EventSequence>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) transactions: Arc<NotificationTransactions>,
    pub(super) comm_state: Arc<AtomicU8>,
    pub(super) max_apdu: u32,
}

fn denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
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
    let devices = db.find_by_type(ObjectType::DEVICE);
    if devices.len() != 1 || devices[0].instance_number() == ObjectIdentifier::MAX_INSTANCE {
        return Err(Error::Encoding(
            "target Audit requires exactly one concrete built-in Device".into(),
        ));
    }
    let device = devices[0];
    let authority = db
        .get_mut(&device)
        .and_then(|object| object.device_authority_internal())
        .filter(|authority| authority.object_identifier() == device)
        .ok_or_else(|| Error::Encoding("target Audit requires a built-in Device".into()))?;
    authority.validate_audit_recipient_installation()?;
    let value = authority
        .provisioned_audit_recipient()
        .cloned()
        .ok_or_else(|| {
            Error::Encoding("target Audit requires a provisioned built-in Device recipient".into())
        })?;
    for selected in &profile.reporters {
        db.get(selected)
            .and_then(|object| object.audit_reporter_internal())
            .filter(|object| object.object_identifier() == *selected)
            .ok_or_else(|| {
                Error::Encoding("invalid audit reporter: missing read capability".into())
            })?;
        db.get_mut(selected).and_then(|object| object.audit_reporter_authority_internal())
            .filter(|object| object.object_identifier() == *selected)
            .ok_or_else(|| Error::Encoding(format!("invalid audit reporter: selected object {selected:?} is absent or lacks the Audit Reporter capability")))?
            .validate_installation()?;
    }
    let route = routes.resolve(&value);
    // A Device may be provisioned before its route is available. Address choices
    // must belong to this runtime's explicit direct-unicast B/IP subset.
    if matches!(value, BACnetRecipient::Address(_)) && route.is_none() {
        return Err(denied());
    }
    for selected in &profile.reporters {
        db.get(selected)
            .unwrap()
            .audit_reporter_internal()
            .unwrap()
            .status_internal()
            .set_configured(route.is_some());
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
        comm_state: &Arc<AtomicU8>,
    ) -> Result<Option<Arc<Self>>, Error> {
        let Some(profile) = &config.audit_reporters else {
            return Ok(None);
        };
        let device = db.find_by_type(ObjectType::DEVICE)[0];
        let association = TargetAuditAssociation::new(
            profile
                .reporters
                .iter()
                .map(|oid| {
                    (
                        *oid,
                        db.get(oid)
                            .unwrap()
                            .audit_reporter_internal()
                            .unwrap()
                            .status_internal(),
                    )
                })
                .collect(),
        );
        let recipient = db
            .get_mut(&device)
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .provisioned_audit_recipient()
            .unwrap()
            .clone();
        let runtime = Arc::new(Self {
            owner: AuditOwnership::for_target(device, Arc::clone(&association)),
            device,
            association: Arc::clone(&association),
            current_route: std::sync::Mutex::new(routes.resolve(&recipient)),
            routes: Arc::clone(&routes),
            sequence: db.event_sequence_internal(),
            network: Arc::clone(network),
            transactions: Arc::clone(transactions),
            comm_state: Arc::clone(comm_state),
            max_apdu: config.max_apdu_length,
        });
        let sink: Arc<dyn AuditRecipientChangeSink> = runtime.clone();
        db.get_mut(&device)
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .install_audit_recipient(&sink)?;
        let reporter_sink: Arc<dyn AuditReporterChangeSink> = runtime.clone();
        for selected in &profile.reporters {
            db.get_mut(selected)
                .unwrap()
                .audit_reporter_authority_internal()
                .ok_or_else(denied)?
                .install(&reporter_sink, &runtime.owner)?;
        }
        transactions.install_target_audit(association);
        db.protect_audit_internal(&runtime.owner)?;
        assert!(transactions.audit_routes.set(routes).is_ok());
        transactions.set_audit_owner(&runtime.owner);
        Ok(Some(runtime))
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
                .and_then(|object| object.audit_reporter_authority_internal())
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
            if !self.owner.is_active() || self.comm_state.load(Ordering::Acquire) != 0 {
                return Err(denied());
            }
            let old_route = self.routes.resolve(current).ok_or_else(denied)?;
            let new_route = self.routes.resolve(&new).ok_or_else(denied)?;
            let mut old_value = BytesMut::new();
            let mut new_value = BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(&mut old_value, current);
            bacnet_encoding::constructed::encode_recipient(&mut new_value, &new);
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
            status.commit_recipient_change(|confirmed, token| {
                let mut attempts = Vec::with_capacity(2);
                for route in [old_route, new_route] {
                    let permit = self.transactions.try_admit_audit().map_err(|_| denied())?;
                    let reservation = if confirmed {
                        Some(
                            self.transactions
                                .reserve(
                                    route.canonical_peer.clone(),
                                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                                )
                                .map_err(|_| denied())?,
                        )
                    } else {
                        None
                    };
                    let invoke = reservation
                        .as_ref()
                        .map_or(0, |(operation, _)| operation.invoke_id());
                    let bytes =
                        encode_notification(&notification, confirmed, self.max_apdu, invoke)
                            .ok_or_else(denied)?;
                    attempts.push((route, permit, reservation, bytes));
                }
                // No fallible work remains. Worker registration happens under
                // the same owner lock as this commit and shutdown sealing.
                *self.current_route.lock().unwrap() = Some(Arc::clone(&attempts[1].0));
                *current = new;
                for (_, other) in self.association.reporters() {
                    if !Arc::ptr_eq(other, &status) {
                        other.recipient_changed_internal();
                    }
                }
                let network = Arc::clone(&self.network);
                let comm_state = Arc::clone(&self.comm_state);
                let status = Arc::clone(&status);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                let mut attempts = attempts.into_iter();
                let first = attempts.next().unwrap();
                let second = attempts.next().unwrap();
                let completion_a = DeliveryCompletion {
                    status: Arc::clone(&status),
                    epoch: token,
                    finished: false,
                };
                let completion_b = DeliveryCompletion {
                    status,
                    epoch: token,
                    finished: false,
                };
                Ok(Some(async move {
                    let run = |(route, _permit, reservation, bytes): (
                        Arc<super::event_recipient_route::ConfirmedRecipientRoute>,
                        tokio::sync::OwnedSemaphorePermit,
                        Option<super::notification_transactions::NotificationReservation>,
                        BytesMut,
                    ),
                               completion: DeliveryCompletion| {
                        let network = Arc::clone(&network);
                        let comm_state = Arc::clone(&comm_state);
                        async move {
                            let _permit = _permit;
                            let delivered = deliver(
                                &network,
                                &comm_state,
                                &route,
                                &bytes,
                                reservation,
                                deadline,
                            )
                            .await;
                            completion.finish(delivered);
                        }
                    };
                    tokio::join!(run(first, completion_a), run(second, completion_b));
                }))
            })
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

pub(super) fn spawn_owned(
    owner: Option<Arc<bacnet_objects::database::AuditOwnership>>,
    future: impl std::future::Future<Output = ()> + Send + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _owner = owner;
        future.await;
    })
}
