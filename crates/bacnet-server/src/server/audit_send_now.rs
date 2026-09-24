//! External command capture and the queue's locally-unsent prefix fence.
use super::audit_reporter::{deliver, encode_notification, small_value, DeliveryCompletion};
use super::*;
use bacnet_objects::{audit::AuditReporterStatus, clock::ClockFrame, device::AuditWriteSource};
use bacnet_types::{
    constructed::{AuditPropertyReference, BACnetAuditNotification, BACnetRecipient},
    enums::AuditOperation,
    primitives::BACnetTimeStamp,
};

impl<T: TransportPort + 'static> super::audit_recipient::TargetAudit<T> {
    pub(super) fn command_send_now(
        &self,
        reporter: ObjectIdentifier,
        status: &Arc<AuditReporterStatus>,
        value: bool,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error> {
        let old = status.send_now();
        let selected = self
            .association
            .select_change(reporter, None)
            .filter(|selected| {
                status.configuration().enabled()
                    || bacnet_objects::audit::ObjectAuditPolicy::default()
                        .effective_internal(&selected.configuration)
                        .reports(
                            AuditOperation::WRITE,
                            Some(PropertyIdentifier::SEND_NOW),
                            None,
                        )
            })
            .filter(|_| value || old);
        let Some(selected) = selected else {
            return self.transactions.commit_audit_without_worker(|| {
                if !self.owner.is_active() {
                    return Err(super::audit_recipient::denied());
                }
                self.batches.command(status, value, false, || Ok(()))
            });
        };
        let apply = |sequence| {
            self.transactions.commit_audit(|| {
                if !self.owner.is_active() || self.comm_state.load(Ordering::Acquire) != 0 {
                    return Err(super::audit_recipient::denied());
                }
                self.batches.command(status, value, true, || {
                    let route = self
                        .current_route
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or_else(super::audit_recipient::denied)?;
                    let permit = self
                        .transactions
                        .try_admit_audit()
                        .map_err(|_| super::audit_recipient::denied())?;
                    let confirmed = selected.configuration.confirmed;
                    let reserved = if confirmed {
                        Some(
                            self.transactions
                                .reserve(
                                    route.canonical_peer.clone(),
                                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                                )
                                .map_err(|_| super::audit_recipient::denied())?,
                        )
                    } else {
                        None
                    };
                    let timestamp = clock
                        .filter(|frame| frame.is_valid_actual_datetime())
                        .map_or(BACnetTimeStamp::SequenceNumber(sequence), |frame| {
                            BACnetTimeStamp::DateTime {
                                date: frame.local_date,
                                time: frame.local_time,
                            }
                        });
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
                            property_identifier: PropertyIdentifier::SEND_NOW,
                            property_array_index: None,
                        }),
                        target_priority: None,
                        target_value: small_value(&PropertyValue::Boolean(value)),
                        current_value: small_value(&PropertyValue::Boolean(old)),
                        result: None,
                    };
                    let bytes = encode_notification(
                        &notification,
                        confirmed,
                        self.max_apdu,
                        reserved
                            .as_ref()
                            .map_or(0, |(operation, _)| operation.invoke_id()),
                    )
                    .ok_or_else(super::audit_recipient::denied)?;
                    let completion = DeliveryCompletion {
                        status: Arc::clone(&selected.status),
                        epoch: selected.status.begin_delivery(),
                        finished: false,
                    };
                    let network = Arc::clone(&self.network);
                    let comm_state = Arc::clone(&self.comm_state);
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    Ok(async move {
                        let _permit = permit;
                        let delivered =
                            deliver(&network, &comm_state, &route, &bytes, reserved, deadline)
                                .await;
                        completion.finish(delivered);
                    })
                })
            })
        };
        if clock.is_some_and(|frame| frame.is_valid_actual_datetime()) {
            apply(0)
        } else {
            self.sequence.transaction(apply)
        }
    }
}
