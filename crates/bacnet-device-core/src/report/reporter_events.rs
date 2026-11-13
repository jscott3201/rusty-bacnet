//! The [`Reporter`]'s event path (#2303): intrinsic reporting after each
//! write and change, the sweep, the fault pass, AcknowledgeAlarm and
//! GetEventInformation, strung together as `bacnet-server` runs them, with
//! each event notification queued in the outbox ahead of the COV
//! notifications of the same change.
//!
//! The schedule follows the server's intrinsic task: a sweep at the
//! configured period, the first at the first [`Reporter::on_deadline`], and
//! between those a sweep when the earliest Time_Delay ends, no sooner than
//! [`SWEEP_FLOOR`] after the last one. Unlike the server's task the
//! reporter is told of every write and change, so it takes each object's
//! due instant then, measured after the last sweep's `now` (decision 20):
//! a period longer than the shortest delay delays no transition.

use alloc::vec::Vec;

use bacnet_encoding::apdu::{AbortPdu, Apdu, ComplexAck, SimpleAck, decode_apdu};
use bacnet_encoding::constructed::{ALL_NOTIFICATION_FORMS, encode_event_notification_of};
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::RecipientLookupOutcome;
use bacnet_objects::traits::no_alarm_configured;
use bacnet_time::{Duration, Instant};
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, EventState, ObjectType,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::super::{ReportConfig, SWEEP_FLOOR};
use super::{Reporter, Tail};
use crate::cov::life_safety::LifeSafetyCovSnapshots;
use crate::cov::{Delivery, Token};
use crate::event::{
    DistributionOutcome, EventInformationFailure, GetEventInformationBudget, INTRINSIC_FORMS,
    MessageTexts, NotificationTransition, RecipientWindow, acknowledge, after_write,
    distribute_event, fault_pass, handle_acknowledge_alarm,
    handle_get_event_information_configured, network_priority, next_due,
    read_committed_history_snapshot, sweep,
};
use crate::reply::{self, SEGMENTED, emit};
use crate::responder::{Inbound, Outcome, Written};

/// The event-value forms the reporter's notifications carry: those
/// intrinsic reporting builds, so an image links no other form's encoder
/// (#2524), and with `event-enrollment` every form, for the enrollment
/// pass to distribute (#2319). An acknowledgement's notification carries
/// none.
const REPORTED_FORMS: u32 = if cfg!(feature = "event-enrollment") {
    ALL_NOTIFICATION_FORMS
} else {
    INTRINSIC_FORMS
};
// Every form intrinsic reporting builds goes out: one left out of the mask
// would be refused at the encoder and counted dropped.
const _: () = assert!(REPORTED_FORMS & INTRINSIC_FORMS == INTRINSIC_FORMS);

/// The event path's schedule.
#[derive(Debug)]
pub(super) struct Events {
    /// The sweep's period, at least [`SWEEP_FLOOR`].
    period: Duration,
    /// The fault pass's period, if it runs.
    fault: Option<Duration>,
    next_sweep: Instant,
    last_sweep: Instant,
    /// The earliest Time_Delay end after the last sweep.
    due: Option<Instant>,
    next_fault: Instant,
    /// Event notifications queued so far, which number their tokens.
    queued: u64,
}

impl Events {
    /// The schedule `config` asks for, if its sweep turns the path on.
    pub(super) fn new(config: &ReportConfig) -> Option<Self> {
        Some(Self {
            period: config.sweep?.max(SWEEP_FLOOR),
            fault: config.fault_detection.map(|period| period.max(SWEEP_FLOOR)),
            next_sweep: Instant::ZERO,
            last_sweep: Instant::ZERO,
            due: None,
            next_fault: Instant::ZERO,
            queued: 0,
        })
    }

    /// When the next sweep runs: the next periodic one, or sooner the
    /// earliest due instant, floored a [`SWEEP_FLOOR`] after the last; or
    /// the fault pass, when it comes first.
    pub(super) fn deadline(&self) -> Option<Instant> {
        bacnet_time::earliest([Some(self.wake()), self.fault.map(|_| self.next_fault)])
    }

    fn wake(&self) -> Instant {
        match self.due {
            Some(due) if due < self.next_sweep => due
                .max(self.last_sweep.saturating_add(SWEEP_FLOOR))
                .min(self.next_sweep),
            _ => self.next_sweep,
        }
    }

    /// A sweep at `now`: on or after the periodic instant it moves the
    /// cadence on, a period after that instant when on time and after
    /// `now` when late, so a stall runs one sweep, not one per period
    /// missed; one woken for a due instant leaves it.
    fn swept(&mut self, now: Instant) {
        self.last_sweep = now;
        if now >= self.next_sweep {
            let from = if now > self.next_sweep {
                now
            } else {
                self.next_sweep
            };
            self.next_sweep = from.saturating_add(self.period);
        }
    }

    /// Take `due`, an object's due instant after a change, if it is after
    /// the last sweep.
    fn seeded(&mut self, due: Option<Instant>) {
        let due = due.filter(|due| *due > self.last_sweep);
        self.due = bacnet_time::earliest([self.due, due]);
    }
}

impl Reporter {
    /// AcknowledgeAlarm or GetEventInformation, when the event path is on:
    /// the reply in `out`, and an accepted acknowledgement's notification
    /// queued after it. `None` for any other request, which the caller
    /// runs.
    pub(super) fn event_service(
        &mut self,
        db: &mut ObjectDatabase,
        request: &Inbound,
        now: Instant,
        out: &mut BytesMut,
    ) -> Option<Written> {
        self.events.as_ref()?;
        let service = event_service(&request.apdu)?;
        Some(if service == ConfirmedServiceChoice::ACKNOWLEDGE_ALARM {
            self.acknowledge_alarm(db, request, now, out)
        } else {
            get_event_information(db, request, out)
        })
    }

    /// AcknowledgeAlarm: the reply in `out`, then an accepted
    /// acknowledgement's notification queued. Out of line, as
    /// [`get_event_information`] is, so [`handle`](Self::handle) carries
    /// neither (#2524).
    #[inline(never)]
    fn acknowledge_alarm(
        &mut self,
        db: &mut ObjectDatabase,
        request: &Inbound,
        now: Instant,
        out: &mut BytesMut,
    ) -> Written {
        let service = ConfirmedServiceChoice::ACKNOWLEDGE_ALARM;
        let Ok(Apdu::ConfirmedRequest(confirmed)) = decode_apdu(request.apdu.clone()) else {
            return Outcome::Silent.into();
        };
        let invoke_id = confirmed.invoke_id;
        let (answer, notification) = match handle_acknowledge_alarm(db, &confirmed.service_request)
        {
            Ok(accepted) => {
                let ack = SimpleAck {
                    invoke_id,
                    service_choice: service,
                };
                (Apdu::SimpleAck(ack), accepted.into_notification())
            }
            Err(error) => (reply::error_apdu(invoke_id, service, &error), None),
        };
        let outcome = emit(out, &answer, Outcome::Reply);
        // After the SimpleACK, as the server sends it.
        if let Some((oid, transition)) = notification {
            self.distribute(db, oid, transition, now);
        }
        outcome.into()
    }

    /// Acknowledge `object`'s latest transition to `state` by local means, a
    /// reset button say (Clause 13.2.3): AcknowledgeAlarm's own path
    /// ([`acknowledge`]), stamped with that transition's committed
    /// timestamp, so its acknowledgement notification, queued here, is the
    /// one the service would send. Refused as the service refuses:
    /// UNKNOWN_OBJECT, NO_ALARM_CONFIGURED, INVALID_EVENT_STATE (`state`
    /// isn't the one the transition reached), or SERVICES /
    /// INVALID_TIMESTAMP, as a mismatched timestamp is, when the object keeps no timestamp for it. With
    /// the event path off it acknowledges, and queues nothing.
    pub fn acknowledge_local(
        &mut self,
        db: &mut ObjectDatabase,
        object: ObjectIdentifier,
        state: EventState,
        now: Instant,
    ) -> Result<(), Error> {
        self.last = now;
        let unknown = Error::protocol(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
        let stored = db.get(&object).ok_or(unknown)?;
        let coordinate = EventTransition::for_target_state(state);
        let timestamp = read_committed_history_snapshot(stored, coordinate, MessageTexts::Optional)
            .map(|history| history.timestamp);
        let Some(timestamp) = timestamp else {
            stored.event_reporting().ok_or_else(no_alarm_configured)?;
            return Err(Error::protocol(
                ErrorClass::SERVICES,
                ErrorCode::INVALID_TIMESTAMP,
            ));
        };
        let accepted = acknowledge(db, object, state, &timestamp)?;
        if self.events.is_some()
            && let Some((oid, transition)) = accepted.into_notification()
        {
            self.distribute(db, oid, transition, now);
        }
        Ok(())
    }

    /// With the event path on, evaluate `objects`' intrinsic reporting after
    /// a change, queue the event notifications it fires and take the due
    /// instants it seeds; the objects whose transition committed come back.
    /// The caller reports `objects` to COV after this, so a committed
    /// transition's Status_Flags change follows its notifications (#889).
    pub(super) fn object_events(
        &mut self,
        db: &mut ObjectDatabase,
        objects: &[ObjectIdentifier],
        now: Instant,
    ) -> Vec<ObjectIdentifier> {
        let mut committed = Vec::new();
        if self.events.is_none() {
            return committed;
        }
        for oid in objects {
            let fired = after_write(db, oid, now);
            let due = db
                .get(oid)
                .and_then(|object| object.event_reporting())
                .and_then(|reporting| reporting.next_due());
            if let Some(events) = self.events.as_mut() {
                events.seeded(due);
            }
            let Some(fired) = fired else {
                continue;
            };
            committed.push(*oid);
            if let Ok(transition) = fired
                && transition.distribute
                && transition.event_values.is_some()
            {
                self.distribute(db, *oid, transition.into(), now);
            }
        }
        committed
    }

    /// The event path's work due at `now`: the fault pass, then the sweep,
    /// each with the COV notifications of what it changed.
    pub(super) fn event_deadline(&mut self, db: &mut ObjectDatabase, now: Instant) {
        let Some(events) = self.events.as_mut() else {
            return;
        };
        let fault = events
            .fault
            .filter(|_| now >= events.next_fault)
            .inspect(|period| events.next_fault = now.saturating_add(*period))
            .is_some();
        let swept = now >= events.wake();
        if swept {
            events.swept(now);
        }
        if fault {
            self.fault_pass(db, now);
        }
        if swept {
            self.sweep(db, now);
        }
    }

    /// Sweep every object at `now`, queue what fires, and report each
    /// proposal's object to COV after the event notifications: a proposal
    /// may commit with its notification refused.
    fn sweep(&mut self, db: &mut ObjectDatabase, now: Instant) {
        let mut snapshots = LifeSafetyCovSnapshots::default();
        let mut changed = Vec::new();
        let swept = sweep(db, now, |db, oid| {
            snapshots.capture(db, oid);
            if !changed.contains(&oid) {
                changed.push(oid);
            }
        });
        for (oid, fired) in swept {
            if let Ok(transition) = fired
                && transition.distribute
                && transition.event_values.is_some()
            {
                self.distribute(db, oid, transition.into(), now);
            }
        }
        let due = next_due(db, now);
        if let Some(events) = self.events.as_mut() {
            events.due = due;
        }
        self.report_committed(db, &changed, &snapshots, now);
    }

    /// Run every object's reliability evaluation and report what changed
    /// to COV; the sweep turns a changed Reliability into its transition.
    fn fault_pass(&mut self, db: &mut ObjectDatabase, now: Instant) {
        // The hooks change objects in place: Life Safety values first.
        let life_safety = db
            .find_by_type(ObjectType::LIFE_SAFETY_POINT)
            .into_iter()
            .chain(db.find_by_type(ObjectType::LIFE_SAFETY_ZONE));
        let snapshots = LifeSafetyCovSnapshots::capture_oids(db, life_safety);
        let pass = fault_pass(db);
        let changed: Vec<_> = pass.changes.iter().map(|change| change.object_id).collect();
        self.report_committed(db, &changed, &snapshots, now);
    }

    /// Report `objects`, which a background pass changed, to COV, through
    /// the tail every change runs.
    fn report_committed(
        &mut self,
        db: &mut ObjectDatabase,
        objects: &[ObjectIdentifier],
        snapshots: &LifeSafetyCovSnapshots,
        now: Instant,
    ) {
        if !objects.is_empty() {
            self.after_change(db, objects, snapshots, Tail::Committed, now);
        }
    }

    /// Distribute `transition` of `oid`: one notification per recipient its
    /// Notification Class selects, each with its process identifier, at the
    /// network priority Table 13-6 gives its event priority. The window is
    /// the Device clock's when the `Responder` has one, judged by its
    /// synchronized state. Without one only destinations open every day,
    /// all day, are selected, whatever clock reader the database holds: a
    /// host that lends one without `Responder::with_clock`, an
    /// unsynchronized `DeviceClock` say, would otherwise open windows on its
    /// valid 1990 frame for entries the write filter never judged (from the
    /// build, or restored; decision 22).
    fn distribute(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        transition: NotificationTransition,
        now: Instant,
    ) {
        let window = match self.responder.clock() {
            Some(clock) => RecipientWindow::Given(clock.clock().window(now)),
            None => RecipientWindow::Given(None),
        };
        let distribution = distribute_event(db, self.device, &oid, transition, window);
        let DistributionOutcome::Built {
            notification,
            lookup,
            ..
        } = distribution.outcome
        else {
            return;
        };
        let RecipientLookupOutcome::Matched(recipients) = lookup else {
            return;
        };
        let priority = network_priority(notification.priority);
        for (recipient, process_identifier, confirmed) in recipients {
            let mut targeted = notification.clone();
            targeted.process_identifier = process_identifier;
            let mut params = BytesMut::new();
            if encode_event_notification_of::<REPORTED_FORMS>(&targeted, &mut params).is_err() {
                self.counters.events_dropped += 1;
                continue;
            }
            self.queue_event((recipient, priority), confirmed, &params, now);
        }
    }

    /// Queue one event notification. Each COV notification it evicts ends
    /// unsent, its subscription owed a later look; a change measured from
    /// an evicted report is looked at again too (#2508).
    fn queue_event(
        &mut self,
        to: (BACnetRecipient, NetworkPriority),
        confirmed: bool,
        params: &[u8],
        now: Instant,
    ) {
        let Some(events) = self.events.as_mut() else {
            return;
        };
        events.queued += 1;
        let token = Token::event(events.queued);
        let pushed = self.outbox.push_event(token, to, confirmed, now, params);
        for evicted in pushed.evicted {
            self.counters.evicted += 1;
            let Some(key) = evicted.key().cloned() else {
                continue;
            };
            if evicted.confirmed {
                self.end_confirmed(evicted.token, Delivery::NotSent, now);
            } else {
                let owed = self.table.delivered(evicted.token, Delivery::NotSent, now);
                self.looks.extend(owed);
            }
            self.owe(key);
        }
        if !pushed.queued {
            self.counters.events_dropped += 1;
        }
        self.settle();
    }
}

/// GetEventInformation: the page, or the refusal, in `out`. The reporter
/// never segments, so the page fits one APDU. Out of line (#2524).
#[inline(never)]
fn get_event_information(db: &ObjectDatabase, request: &Inbound, out: &mut BytesMut) -> Written {
    let service = ConfirmedServiceChoice::GET_EVENT_INFORMATION;
    let Ok(Apdu::ConfirmedRequest(confirmed)) = decode_apdu(request.apdu.clone()) else {
        return Outcome::Silent.into();
    };
    let invoke_id = confirmed.invoke_id;
    let mut budget = GetEventInformationBudget::default();
    let header = 3;
    let room = usize::from(request.max_reply).saturating_sub(header);
    budget.max_service_ack_bytes = budget.max_service_ack_bytes.min(room);
    let mut ack = BytesMut::new();
    let data = &confirmed.service_request;
    let answer = match handle_get_event_information_configured(db, data, &mut ack, budget) {
        Ok(()) => Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_ack: ack.freeze(),
        }),
        Err(EventInformationFailure::Service(error)) => {
            reply::error_apdu(invoke_id, service, &error)
        }
        Err(EventInformationFailure::Projection(problem)) => {
            reply::error_apdu(invoke_id, service, &problem.to_error())
        }
        Err(failure) => Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason: match failure {
                EventInformationFailure::Objects => AbortReason::OUT_OF_RESOURCES,
                _ => AbortReason::BUFFER_OVERFLOW,
            },
        }),
    };
    emit(out, &reply::fit(answer, request.max_reply), Outcome::Reply).into()
}

/// An event service the reporter runs, read from an unsegmented confirmed
/// request's header.
fn event_service(apdu: &[u8]) -> Option<ConfirmedServiceChoice> {
    let (&first, service) = (apdu.first()?, apdu.get(3)?);
    if first >> 4 != 0 || first & SEGMENTED != 0 {
        return None;
    }
    let service = ConfirmedServiceChoice::from_raw(*service);
    matches!(
        service,
        ConfirmedServiceChoice::ACKNOWLEDGE_ALARM | ConfirmedServiceChoice::GET_EVENT_INFORMATION
    )
    .then_some(service)
}
