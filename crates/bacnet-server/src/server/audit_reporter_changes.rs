//! Atomic local Reporter-property changes through the existing notification owner.
use super::audit_reporter::{deliver, encode_notification, small_value, DeliveryCompletion};
use super::*;
use bacnet_objects::{
    audit::{AuditReporterChangeSink, AuditReporterConfiguration, AuditReporterStatus},
    clock::ClockFrame,
    device::AuditWriteSource,
};
use bacnet_types::{
    constructed::{AuditPropertyReference, BACnetAuditNotification, BACnetRecipient},
    enums::AuditOperation,
    primitives::BACnetTimeStamp,
};

fn denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
}

impl<T: TransportPort + 'static> AuditReporterChangeSink
    for super::audit_recipient::TargetAudit<T>
{
    fn is_active(&self) -> bool {
        self.owner.is_active()
    }
    fn change(
        &self,
        reporter: ObjectIdentifier,
        status: &Arc<AuditReporterStatus>,
        next: AuditReporterConfiguration,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error> {
        let before = status.configuration();
        let use_after = !before.enabled() && next.enabled();
        let elected = self
            .association
            .select_change(reporter, use_after.then_some((reporter, &next)));
        let mut changes = vec![];
        if elected.is_some() {
            for property in [
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::AUDIT_LEVEL,
                PropertyIdentifier::AUDITABLE_OPERATIONS,
                PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
                PropertyIdentifier::MONITORED_OBJECTS,
                PropertyIdentifier::AUDIT_PRIORITY_FILTER,
            ] {
                let old = before.property(property);
                let new = next.property(property);
                let eligible = before.enabled()
                    || next.enabled()
                    || elected.as_ref().is_some_and(|owner| {
                        bacnet_objects::audit::ObjectAuditPolicy::default()
                            .effective_internal(&owner.configuration)
                            .reports(AuditOperation::WRITE, Some(property), None)
                    });
                if old != new && eligible {
                    changes.push((property, old, new));
                }
            }
            changes.sort_by_key(|(property, _, _)| property.to_raw());
        }
        let count = changes.len() as u16;
        let route = self.current_route.lock().unwrap().clone();
        let apply = |sequence: u16| {
            self.transactions.commit_audit(|| {
                if !self.owner.is_active() {
                    return Err(denied());
                }
                if count != 0 && self.comm_state.load(Ordering::Acquire) != 0 {
                    return Err(denied());
                }
                status.commit_configuration(next, |new_token| {
                    let mut attempts = Vec::with_capacity(changes.len());
                    for (offset, (property, old, new)) in changes.into_iter().enumerate() {
                        let elected = elected.as_ref().expect("eligible change");
                        let route = route.clone().ok_or_else(denied)?;
                        let confirmed = elected.configuration.confirmed;
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
                        let timestamp = match clock.filter(|frame| frame.is_valid_actual_datetime())
                        {
                            Some(frame) => BACnetTimeStamp::DateTime {
                                date: frame.local_date,
                                time: frame.local_time,
                            },
                            None => BACnetTimeStamp::SequenceNumber(
                                sequence.wrapping_add(offset as u16),
                            ),
                        };
                        let notification = BACnetAuditNotification {
                            source_timestamp: None,
                            target_timestamp: Some(timestamp),
                            source_device: source
                                .map_or(BACnetRecipient::Device(self.device), |source| {
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
                            target_object: Some(reporter),
                            target_property: Some(AuditPropertyReference {
                                property_identifier: property,
                                property_array_index: None,
                            }),
                            target_priority: None,
                            target_value: new.as_ref().and_then(small_value),
                            current_value: old.as_ref().and_then(small_value),
                            result: None,
                        };
                        let invoke = reservation
                            .as_ref()
                            .map_or(0, |(operation, _)| operation.invoke_id());
                        let bytes =
                            encode_notification(&notification, confirmed, self.max_apdu, invoke)
                                .ok_or_else(denied)?;
                        let token = if Arc::ptr_eq(status, &elected.status) {
                            new_token
                        } else {
                            elected.status.begin_delivery()
                        };
                        attempts.push((
                            route,
                            permit,
                            reservation,
                            bytes,
                            Arc::clone(&elected.status),
                            token,
                        ));
                    }
                    let network = Arc::clone(&self.network);
                    let comm_state = Arc::clone(&self.comm_state);
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    let attempts: Vec<_> = attempts
                        .into_iter()
                        .map(|(route, permit, reservation, bytes, status, epoch)| {
                            (
                                route,
                                permit,
                                reservation,
                                bytes,
                                DeliveryCompletion {
                                    status,
                                    epoch,
                                    finished: false,
                                },
                            )
                        })
                        .collect();
                    Ok((!attempts.is_empty()).then_some(async move {
                        futures_util::future::join_all(attempts.into_iter().map(
                            |(route, permit, reservation, bytes, completion)| {
                                let network = Arc::clone(&network);
                                let comm_state = Arc::clone(&comm_state);
                                async move {
                                    let _permit = permit;
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
                            },
                        ))
                        .await;
                    }))
                })
            })
        };
        if count == 0 || clock.is_some_and(|frame| frame.is_valid_actual_datetime()) {
            apply(0)?;
        } else {
            self.sequence.transaction_many(count, apply)?;
        }
        self.association.refresh_overlap();
        if let Some(queue) = self.transactions.audit_failure_queue(status) {
            queue.recipient_changed();
        }
        Ok(())
    }
}
