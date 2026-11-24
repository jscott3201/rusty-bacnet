//! Source Audit of the client on the session's seat (#2617).
//!
//! The client hands the session every confirmed request of a service Table
//! 19-5 maps to READ or WRITE (see `bacnet_client::client::SourceAuditor`).
//! Here the request is admitted: once its turn at the destination is due,
//! a slot from its lane's share of the 64 ([`super::slots`], #2652) and
//! the turn with it, then the database guard, the Reporter's filters, a
//! recipient, one station, a timestamp. Then a task the session owns runs it, so a caller that goes
//! away loses neither the request nor its records: the client retries with
//! the same invoke ID and the records carry the real answer. A request the
//! Reporter doesn't select, or that has no recipient, runs unaudited in the
//! caller's task.
//!
//! A write sent with a priority is a command at that priority, which the
//! Reporter's Audit_Priority_Filter applies to and the record names; one
//! sent without a priority is not filtered by priority. An operation cut
//! off by a stop, of the session or of the client, is not recorded.
use std::future::Future;
use std::pin::Pin;

use bacnet_client::client::{AuditTarget, AuditedRequest, SourceAuditor};
use bacnet_types::bitstring::BACnetPriorityFilter;
use bytes::Bytes;

use super::*;

/// The session's source Audit, as its client's auditor. It holds the
/// source weakly: a client kept past its session keeps nothing of it.
pub(crate) struct SeatAuditor(pub(crate) Weak<SourceAudit>);

impl SourceAuditor for SeatAuditor {
    fn audit(
        &self,
        request: AuditedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Bytes, Error>> + Send>> {
        let source = self.0.clone();
        Box::pin(async move {
            source
                .upgrade()
                .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?
                .audit_seated(request)
                .await
        })
    }
}

/// What the Reporter selects, as it stood when a request was admitted.
#[derive(Clone, Copy)]
struct Selection {
    level: AuditLevel,
    /// Whether Auditable_Operations holds the request's operation.
    operation: bool,
    /// Audit_Priority_Filter, read when a target was sent with a priority.
    priorities: Option<BACnetPriorityFilter>,
}

impl Selection {
    /// Whether a record of `target` is made.
    fn selects(&self, target: &AuditTarget) -> bool {
        let priority = target.priority.is_none_or(|priority| {
            self.priorities
                .is_none_or(|filter| filter.contains(priority))
        });
        let present_value = target.property.as_ref().is_some_and(|property| {
            property.property_identifier == PropertyIdentifier::PRESENT_VALUE
        });
        self.level != AuditLevel::NONE
            && self.operation
            && priority
            && !(self.level == AuditLevel::AUDIT_CONFIG && present_value)
    }
}

impl SourceAudit {
    /// Admit and carry out a request of the session's client, as the
    /// module documentation describes.
    async fn audit_seated(self: &Arc<Self>, mut request: AuditedRequest) -> Result<Bytes, Error> {
        let lane = slots::Lane::of(request.station());
        let slot = loop {
            // Its turn at the destination (`min_request_interval_ms`) is
            // taken as it is admitted, so the interval holds between
            // requests as they are admitted (#2652). It first waits for the
            // turn to come due, holding nothing, so a request waiting out
            // the interval keeps no other out of its lane.
            request.turn_due().await?;
            // At most 64 operations are held at once, and at most a lane's
            // share by one lane. Past them a request waits here for a slot,
            // in its caller's task: nothing is spawned for it, its timeout
            // starts once it is admitted, and a stop (which closes the
            // slots) or its caller going away ends the wait with nothing
            // sent and no slot held.
            let slot = (self.slots)
                .admit(lane.clone())
                .await
                .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
            // A request that waited for its slot while the node's
            // DeviceCommunicationControl took effect goes no further:
            // refused before it takes a turn or a sequence number, as one
            // made then is.
            if request.initiation_restricted() {
                return Err(Error::InitiationDisabled { sent: false });
            }
            // Another request to the destination may have taken the turn
            // while this one waited for its slot: the slot goes back, and
            // it waits for the next turn.
            if request.try_take_turn() {
                break slot;
            }
        };
        let mut db = self.db.write().await;
        if self.slots.is_closed() {
            return Err(Error::Encoding("endpoint shutdown".into()));
        }
        let runtime = self
            .runtime
            .upgrade()
            .filter(|runtime| runtime.owner.is_active())
            .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
        let reporter = db
            .get(&self.selected)
            .and_then(|object| object.audit_reporter())
            .ok_or_else(|| Error::Encoding("source Reporter is unavailable".into()))?;
        if has_property(
            &reporter.property_metadata(),
            PropertyIdentifier::MONITORED_OBJECTS,
        ) {
            reporter.status_internal().set_configured(false);
            return Err(Error::Encoding(
                "source Audit does not support Monitored_Objects".into(),
            ));
        }
        let level = match reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None)? {
            PropertyValue::Enumerated(level) => AuditLevel::from_raw(level),
            _ => return Err(Error::Encoding("invalid source audit level".into())),
        };
        let operations =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None)? {
                PropertyValue::BitString { unused_bits, data } => {
                    AuditOperationFlags::from_bacnet(unused_bits, &data)?
                }
                _ => return Err(Error::Encoding("invalid source operation flags".into())),
            };
        let priorities = if request
            .targets()
            .iter()
            .any(|target| target.priority.is_some())
        {
            match reporter.read_property(PropertyIdentifier::AUDIT_PRIORITY_FILTER, None)? {
                PropertyValue::BitString { unused_bits, data } => {
                    Some(BACnetPriorityFilter::from_bacnet(unused_bits, &data)?)
                }
                _ => return Err(Error::Encoding("invalid source priority filter".into())),
            }
        } else {
            None
        };
        let selection = Selection {
            level,
            operation: operations.contains(request.operation()),
            priorities,
        };
        if !request
            .targets()
            .iter()
            .any(|target| selection.selects(target))
        {
            drop(db);
            drop(runtime);
            drop(slot);
            return request.execute().await.result;
        }
        let status = reporter.status_internal();
        let confirmed = reporter.confirmed_internal();
        let route = runtime
            .recipient_route(&db)
            .filter(|route| route.serves(confirmed));
        status.set_configured(route.is_some());
        let Some(route) = route else {
            drop(db);
            drop(runtime);
            drop(slot);
            return request.execute().await.result;
        };
        let station = request.station().clone();
        self.one_station(&runtime, station.network_number, &station.mac_address)?;
        let device = self.device;
        if db.local_device().identifier() != Some(device) {
            status.set_configured(false);
            return Err(Error::Encoding("source Device is unavailable".into()));
        }
        // Again just before the sequence number: one that took effect while
        // the request waited for the database is refused here, taking none.
        // From here to the request's registration, in the session's task,
        // the control can still take effect: the request is then refused
        // there, unsent and unrecorded, and its number is left unused.
        if request.initiation_restricted() {
            return Err(Error::InitiationDisabled { sent: false });
        }
        let timestamp = match db
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime())
        {
            Some(frame) => BACnetTimeStamp::DateTime {
                date: frame.local_date,
                time: frame.local_time,
            },
            None => BACnetTimeStamp::SequenceNumber(db.next_event_sequence_number()),
        };
        let failure = status.auditing_failure_epoch().and_then(|epoch| {
            self.failures.observe(AuditFailureContext {
                status: Arc::clone(&status),
                epoch,
                device,
                confirmed,
                peer: route.peer(),
                route: route.clone(),
                max_apdu: u32::from(self.max_apdu),
            })
        });
        let completion = status.begin_delivery();
        let notification = BACnetAuditNotification {
            source_timestamp: Some(timestamp),
            target_timestamp: None,
            source_device: BACnetRecipient::Device(device),
            source_object: None,
            operation: request.operation(),
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Address(station),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        };
        status.set_configured(true);
        drop(db);
        drop(runtime);
        let owner = self
            .notifications
            .upgrade()
            .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
        let weak_owner = Arc::downgrade(&owner);
        let source = Arc::clone(self);
        let (reply, response) = oneshot::channel();
        // A rejected spawn drops the request, its slot and the reply; the
        // worker never retains its owner. The stop that closes the owner
        // aborts the worker, so an operation it cuts off is not recorded.
        owner.spawn(async move {
            let _slot = slot;
            let outcome = request.execute().await;
            // A stop that began while the request ran cut it off.
            let recorded = outcome.attempted && !source.slots.is_closed();
            if let Some(owner) = weak_owner.upgrade().filter(|_| recorded) {
                for (target, result) in &outcome.records {
                    if !selection.selects(target) {
                        continue;
                    }
                    let mut record = notification.clone();
                    record.invoke_id = outcome.invoke_id;
                    record.target_object = Some(target.object);
                    record.target_property = target.property.clone();
                    record.target_priority = target.priority;
                    record.target_value = target.value.clone().filter(|value| value.len() <= 32);
                    record.result = *result;
                    if let Some(device) = outcome.target_device {
                        record.target_device = BACnetRecipient::Device(device);
                    }
                    delivery::admit(
                        &source,
                        &owner,
                        confirmed,
                        route.clone(),
                        record,
                        delivery::Completion::new(Arc::clone(&status), completion),
                        failure.clone(),
                    );
                }
            }
            let _ = reply.send(outcome.result);
        });
        drop(owner);
        response
            .await
            .map_err(|_| Error::Encoding("endpoint shutdown".into()))?
    }
}
