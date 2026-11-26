//! Source Audit of the client on the session's seat (#2617).
//!
//! The client hands the session every confirmed request of a service Table
//! 19-5 maps to READ or WRITE (see `bacnet_client::client::SourceAuditor`).
//! Here the request is admitted: once its turn at the destination is due,
//! a slot from its lane's share of the 64 ([`super::slots`], #2652) and
//! the turn with it. Then, under the database guard, the Reporter's
//! filters, a recipient, one station and a timestamp
//! ([`SourceAudit::plan`]); then a task the session owns carries out the
//! request and records it, so a caller that goes away once its request is
//! planned loses neither the request nor its records: the client retries
//! with the same invoke ID and the records carry the real answer. A request
//! the Reporter doesn't select, or that has no recipient, runs unaudited in
//! its caller's task.
//!
//! The guard is never waited for in the caller's task (#2755): Tokio's lock
//! hands its guard to a queued waiter whether or not that waiter is polled
//! again, so a caller parked in a `select!`, `join!` or `buffer_unordered`
//! could own it. The caller takes it only with `try_write`, which queues
//! for nothing, in the poll that admits the request: with the database
//! free, the request is planned there, as it always was. With the database
//! busy, the request and its slot go to the task, which waits for the guard
//! and plans the request there; a caller that goes before that task holds
//! the guard loses the request, with nothing sent or recorded. Otherwise
//! the caller's task waits only where nothing is handed to a waiter that
//! isn't polled: for the turn, for a slot (taken as it is polled) and for
//! the task's answer. So a parked caller holds no guard, and from its
//! request's admission no slot: the task holds it while the request runs,
//! and one that runs unaudited has given it back.
//!
//! Sequence numbers follow admission while the database is free. A request
//! admitted while it is busy takes its number when its task gets the guard,
//! so one admitted after it, once the database is free, can take an
//! earlier number. A panic in the task reaches its caller as the shutdown
//! error.
//!
//! A write sent with a priority is a command at that priority, which the
//! Reporter's Audit_Priority_Filter applies to and the record names; one
//! sent without a priority is not filtered by priority. An operation cut
//! off by the session's stop is not recorded. One the client's own stop or
//! drop cut off after it may have gone out is, with COMMUNICATION, OTHER
//! for its unknown outcome (#1833); one cut off before it went is not.
use std::future::Future;
use std::pin::Pin;

use bacnet_client::client::{AuditTarget, AuditedRequest, SourceAuditor};
use bacnet_objects::audit::AuditDeliveryToken;
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
                .ok_or_else(shutdown)?
                .audit_seated(request)
                .await
        })
    }
}

/// What a request cut off by the session's stop or drop fails with: the
/// session is closed for good, as a stopped one never starts again.
fn shutdown() -> Error {
    Error::state(bacnet_types::error::Lifecycle::Closed, "endpoint shutdown")
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

/// What the session's task hands back to an admitted request's caller.
enum Handoff {
    /// The request's end, or its refusal.
    Answer(Result<Bytes, Error>),
    /// A request the Reporter doesn't select, or with no recipient: its
    /// caller runs it, unaudited. Boxed, as it is the larger by far.
    Unaudited(Box<AuditedRequest>),
}

/// How an admitted request is recorded, as the database stood when it was
/// planned under the guard.
struct Plan {
    selection: Selection,
    notification: BACnetAuditNotification,
    status: Arc<AuditReporterStatus>,
    confirmed: bool,
    route: SourceRoute,
    completion: AuditDeliveryToken,
    failure: Option<AuditFailureTicket<SourceRoute>>,
}

impl SourceAudit {
    /// Admit a request of the session's client, plan it, and hand it to a
    /// task the session owns, as the module documentation describes.
    async fn audit_seated(self: &Arc<Self>, mut request: AuditedRequest) -> Result<Bytes, Error> {
        let lane = slots::Lane::of(request.station());
        let slot = loop {
            // Its turn at the destination (`min_request_interval_ms`) is
            // taken with its slot, so the interval holds between requests
            // as they are admitted (#2652). It first waits for the turn to
            // come due, holding nothing, so a request waiting out the
            // interval keeps no other out of its lane.
            request.turn_due().await?;
            // At most 64 operations are held at once, and at most a lane's
            // share by one lane. Past them a request waits here for a slot,
            // in its caller's task: nothing is spawned for it, its timeout
            // starts once it is admitted, and a stop (which closes the
            // slots) or its caller going away ends the wait with nothing
            // sent and no slot held. A slot is taken only with the turn:
            // one another request to the destination took meanwhile sends
            // it back to wait for the next, holding nothing.
            match (self.slots)
                .admit(lane.clone(), || request.try_take_turn())
                .await
            {
                slots::Admission::Admitted(slot) => break slot,
                slots::Admission::NotReady => {}
                slots::Admission::Closed => return Err(shutdown()),
            }
        };
        // A request that waited for its slot while the node's
        // DeviceCommunicationControl took effect goes no further: refused
        // before it takes a sequence number, as one made then is.
        if request.initiation_restricted() {
            return Err(Error::InitiationDisabled { sent: false });
        }
        let owner = self.notifications.upgrade().ok_or_else(shutdown)?;
        let (reply, answer) = oneshot::channel();
        let (task, weak) = (Arc::clone(self), Arc::downgrade(&owner));
        // The guard only if it is free now: `try_write` queues for nothing,
        // so nothing is handed to this caller while it isn't polled. Planned
        // here, the request goes to its task in this same poll, so a caller
        // that goes afterwards leaves it running and recorded, as before.
        let planned = self
            .db
            .try_write()
            .map(|mut db| self.plan(&mut db, &request));
        // A rejected spawn drops the request, its slot and the reply, with
        // nothing sent; the task never retains its owner. The stop that
        // closes the owner aborts the task, so an operation it cuts off is
        // not recorded.
        let spawned = match planned {
            Ok(Ok(Some(plan))) => owner.spawn(task.carry_out(plan, request, slot, reply, weak)),
            Ok(Ok(None)) => {
                drop((slot, owner));
                return request.execute().await.result;
            }
            Ok(Err(error)) => return Err(error),
            Err(_busy) => owner.spawn(task.run_admitted(request, slot, reply, weak)),
        };
        drop(owner);
        if !spawned {
            return Err(shutdown());
        }
        match answer.await {
            Ok(Handoff::Answer(result)) => result,
            Ok(Handoff::Unaudited(request)) => (*request).execute().await.result,
            // The stop aborted the task, which may have sent the request
            // already: whether the device carried it out is unknown (#1833).
            Err(_) => Err(Error::RequestCutOff { sent: true }),
        }
    }

    /// A request admitted while the database was busy, in the task the
    /// session owns (#2755). It waits for the guard and holds it, polled,
    /// only for the synchronous [`Self::plan`], then goes on as
    /// [`Self::carry_out`]. A caller that goes before the guard is held ends
    /// the request there, with nothing sent, recorded or numbered, and its
    /// slot back at once.
    async fn run_admitted(
        self: Arc<Self>,
        request: AuditedRequest,
        slot: slots::Slot,
        mut reply: oneshot::Sender<Handoff>,
        owner: Weak<NotificationTransactions>,
    ) {
        let plan = {
            let Some(mut db) = self.write_for(&mut reply).await else {
                return;
            };
            self.plan(&mut db, &request)
        };
        match plan {
            Ok(Some(plan)) => self.carry_out(plan, request, slot, reply, owner).await,
            Ok(None) => {
                drop(slot);
                let _ = reply.send(Handoff::Unaudited(Box::new(request)));
            }
            Err(error) => {
                drop(slot);
                let _ = reply.send(Handoff::Answer(Err(error)));
            }
        }
    }

    /// A planned request, in the task the session owns: it is carried out,
    /// recorded as `plan` says, and answered, holding its slot to its end.
    async fn carry_out(
        self: Arc<Self>,
        plan: Plan,
        request: AuditedRequest,
        slot: slots::Slot,
        reply: oneshot::Sender<Handoff>,
        owner: Weak<NotificationTransactions>,
    ) {
        let outcome = request.execute().await;
        // A stop that began while the request ran cut it off.
        let recorded = outcome.attempted && !self.slots.is_closed();
        if let Some(owner) = owner.upgrade().filter(|_| recorded) {
            for (target, result) in &outcome.records {
                if !plan.selection.selects(target) {
                    continue;
                }
                let mut record = plan.notification.clone();
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
                    &self,
                    &owner,
                    plan.confirmed,
                    plan.route.clone(),
                    record,
                    delivery::Completion::new(Arc::clone(&plan.status), plan.completion),
                    plan.failure.clone(),
                );
            }
        }
        let _ = reply.send(Handoff::Answer(outcome.result));
        drop(slot);
    }

    /// Under the database guard, with nothing awaited: the Reporter's
    /// settings and whether they select the request, its recipient and
    /// station, and its timestamp, which takes a sequence number when the
    /// clock has no valid time (why the guard is a write guard). `None` for
    /// a request that goes unaudited.
    fn plan(
        &self,
        db: &mut ObjectDatabase,
        request: &AuditedRequest,
    ) -> Result<Option<Plan>, Error> {
        // A stop is seen here: `close` seals the owner before the slots.
        let runtime = self
            .runtime
            .upgrade()
            .filter(|runtime| runtime.owner.is_active())
            .ok_or_else(shutdown)?;
        let reporter = db
            .get(&self.selected)
            .and_then(|object| object.audit_reporter())
            .ok_or(Error::invalid_config("source Reporter is unavailable"))?;
        if has_property(
            &reporter.property_metadata(),
            PropertyIdentifier::MONITORED_OBJECTS,
        ) {
            reporter.status_internal().set_configured(false);
            return Err(Error::unsupported(
                "source Audit does not support Monitored_Objects",
            ));
        }
        let level = match reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None)? {
            PropertyValue::Enumerated(level) => AuditLevel::from_raw(level),
            _ => return Err(Error::invalid_config("invalid source audit level")),
        };
        let operations =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None)? {
                PropertyValue::BitString { unused_bits, data } => {
                    AuditOperationFlags::from_bacnet(unused_bits, &data)?
                }
                _ => return Err(Error::invalid_config("invalid source operation flags")),
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
                _ => return Err(Error::invalid_config("invalid source priority filter")),
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
            return Ok(None);
        }
        let status = reporter.status_internal();
        let confirmed = reporter.confirmed_internal();
        let route = runtime
            .recipient_route(db)
            .filter(|route| route.serves(confirmed));
        status.set_configured(route.is_some());
        let Some(route) = route else {
            return Ok(None);
        };
        let station = request.station().clone();
        self.one_station(&runtime, station.network_number, &station.mac_address)?;
        let device = self.device;
        if db.local_device().identifier() != Some(device) {
            status.set_configured(false);
            return Err(Error::invalid_config("source Device is unavailable"));
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
        Ok(Some(Plan {
            selection,
            notification,
            status,
            confirmed,
            route,
            completion,
            failure,
        }))
    }
}
