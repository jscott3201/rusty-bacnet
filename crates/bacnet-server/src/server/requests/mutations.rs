use super::*;
use crate::command_lists::TakenRuns;
use crate::mutation::{
    MutationAuthorizationContext, MutationDecision, MutationDecisions, MutationPolicy,
    MutationTarget, MutationTrust,
};
use crate::server::event_notifications::{commit_written_transition, committing};
use crate::server::event_send::Settle;
use bacnet_device_core::cov::life_safety::LifeSafetyCovChange;
use bacnet_device_core::write::{self, WpmOutcome};
use bacnet_objects::staging::StagingWritePlan;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_services::file::AtomicWriteFileRequest;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::write_property::WritePropertyRequest;

mod object_lifecycle;

/// The pass of each Schedule among `written`, objects a request's write
/// committed to under `db`, its guard ([`crate::schedule::reevaluate_written`]):
/// the targets the pass writes commit their transitions under that guard,
/// counted in the request's `settle` (#2535). Boxed, and only for a
/// Schedule: a write to one is rare, and the pass's future would size every
/// write's.
async fn reevaluate_schedules<T: TransportPort + 'static>(
    services: &RequestServices<T>,
    db: &mut ObjectDatabase,
    written: &[ObjectIdentifier],
    settle: &mut Settle,
) -> crate::committed_cov::CommittedCov {
    let schedule = |oid: &ObjectIdentifier| oid.object_type() == ObjectType::SCHEDULE;
    if !written.iter().any(schedule) {
        return Default::default();
    }
    let mut transition = committing(|| services.event_delivery(), settle);
    let (database, table) = (&services.db, &services.cov_table);
    Box::pin(crate::schedule::reevaluate_written(
        database,
        db,
        written,
        table,
        &mut transition,
    ))
    .await
}

pub(super) enum InitialCovNotification {
    Single(Box<CovSubscriptionSnapshot>),
    Multiple(Vec<CovSubscriptionSnapshot>),
}

/// Post-write work a mutating service collects while it runs, consumed after
/// the response is built: event transitions to send, COV changes, staged
/// writes to execute and Command runs to start.
#[derive(Default)]
pub(super) struct MutationEffects {
    /// The event notifications the writes' transitions pushed onto their
    /// recipients' lanes under the writes' own guard (#2530, #2537), for the
    /// COV fanout to wait on (#889).
    pub(super) settle: Settle,
    pub(super) coarse_cov_oids: Vec<ObjectIdentifier>,
    pub(super) life_safety_cov_changes: Vec<LifeSafetyCovChange>,
    pub(super) staging_plans: Vec<StagingWritePlan>,
    /// Command lists and Channel distributions a Present_Value write
    /// started (#1150), ended if dropped before they start (#1324).
    pub(super) command_runs: TakenRuns,
    /// Timestamped references to evaluate again after the post-write fanout,
    /// which may not have selected them (#856).
    pub(super) timed_revisits: Vec<bacnet_device_core::cov::CovSubscriptionKey>,
}

/// Borrowed dispatch inputs; constructed only after the DCC precheck.
/// `provenance` is the reassembled ingress snapshot (fail-closed at
/// reassembly on cross-segment mismatch), threaded unchanged into every
/// authorization context built from this request.
pub(super) struct Request<'a> {
    pub config: &'a ServerConfig,
    pub decisions: &'a MutationDecisions,
    pub source_mac: &'a [u8],
    pub source_network: Option<&'a NpduAddress>,
    pub provenance: bacnet_transport::port::TransportProvenance,
    pub req: &'a ConfirmedRequestPdu,
    pub command_origin: Option<&'a bacnet_objects::command_source::CommandOrigin>,
}

/// Ask `policy` about each of `contexts` in turn, up to and including the
/// first it denies, and return its answers (#2032).
///
/// The calls run on Tokio's blocking pool while the request task awaits
/// them, holding no database guard. A policy that blocks (a Python callable
/// waiting on another thread, say) then holds no runtime worker, and the
/// request task can still be aborted: `stop()`, which joins every admitted
/// request, ends without waiting for the call. An abandoned call finishes on
/// its own thread and its answer is dropped. A panic denies, as in
/// [`fail_closed_authorize`](audit_notification::fail_closed_authorize), and
/// so does a call that never reports back.
///
/// A policy that [`decides_inline`](crate::ServerPolicy::decides_inline) is
/// asked right here on the request task instead, with no trip (#2049);
/// `stop()` then waits for these calls, which that method's rules keep
/// short.
pub(in crate::server) async fn ask_mutation_policy(
    policy: &Arc<dyn crate::ServerPolicy>,
    contexts: Vec<MutationAuthorizationContext>,
) -> Vec<bool> {
    if policy.decides_inline() {
        return ask_in_turn(&**policy, &contexts);
    }
    let policy = Arc::clone(policy);
    let asked = contexts.len().min(1);
    tokio::task::spawn_blocking(move || ask_in_turn(&*policy, &contexts))
        .await
        .unwrap_or_else(|_| vec![false; asked])
}

/// [`ask_mutation_policy`]'s calls, on whichever thread it chose.
fn ask_in_turn(
    policy: &dyn crate::ServerPolicy,
    contexts: &[MutationAuthorizationContext],
) -> Vec<bool> {
    let mut answers = Vec::with_capacity(contexts.len());
    for context in contexts {
        let allowed =
            audit_notification::fail_closed_authorize(|| policy.authorize_mutation(context));
        answers.push(allowed);
        if !allowed {
            break;
        }
    }
    answers
}

impl Request<'_> {
    async fn authorize(
        &self,
        decode: impl FnOnce() -> Result<MutationTarget, Error>,
    ) -> Result<(), Error> {
        if self.config.mutation_policy == MutationPolicy::Permissive
            && self.config.mutation_decider().is_none()
        {
            return self.record(MutationDecision::Allow);
        }
        let target = decode().map_err(Error::into_request_reject)?;
        let decision = self.decide(vec![target]).await[0];
        self.record(decision)
    }

    /// The mode's answer when no policy is asked: `DenyAll` denies, and
    /// without a policy that decides mutations everything is allowed.
    fn decide_without_policy(&self) -> Option<MutationDecision> {
        if self.config.mutation_policy == MutationPolicy::DenyAll {
            return Some(MutationDecision::PolicyDeny);
        }
        self.config
            .mutation_decider()
            .is_none()
            .then_some(MutationDecision::Allow)
    }

    /// What policy decides on each of `targets` in turn, up to and including
    /// the first it denies, asking the installed
    /// [`ServerPolicy`](crate::ServerPolicy) unless the mode answers first
    /// ([`ask_mutation_policy`]). Never empty when `targets` isn't. Nothing
    /// is recorded.
    async fn decide(&self, targets: Vec<MutationTarget>) -> Vec<MutationDecision> {
        if let Some(decision) = self.decide_without_policy() {
            return if decision == MutationDecision::Allow {
                vec![decision; targets.len()]
            } else {
                vec![decision; targets.len().min(1)]
            };
        }
        let policy = self.config.mutation_decider().expect("asked above");
        let contexts = targets
            .into_iter()
            .map(|target| MutationAuthorizationContext {
                source_mac: MacAddr::from_slice(self.source_mac),
                source_network: self.source_network.cloned(),
                provenance: self.provenance.clone(),
                trust: MutationTrust::from_provenance(&self.provenance),
                invoke_id: Some(self.req.invoke_id),
                service_choice: self.req.service_choice.into(),
                target,
            })
            .collect();
        ask_mutation_policy(policy, contexts)
            .await
            .into_iter()
            .map(|allowed| {
                if allowed {
                    MutationDecision::Allow
                } else {
                    MutationDecision::Deny
                }
            })
            .collect()
    }

    /// Count `decision` and answer the request with it.
    fn record(&self, decision: MutationDecision) -> Result<(), Error> {
        self.decisions.record(self.req.service_choice, decision);
        match decision {
            MutationDecision::Allow => Ok(()),
            MutationDecision::Deny | MutationDecision::PolicyDeny => {
                Err(audit_notification::request_denied())
            }
        }
    }

    fn error(&self, error: &Error) -> Apdu {
        bacnet_device_core::reply::error_apdu(self.req.invoke_id, self.req.service_choice, error)
    }

    fn simple_ack(&self) -> Apdu {
        Apdu::SimpleAck(SimpleAck {
            invoke_id: self.req.invoke_id,
            service_choice: self.req.service_choice,
        })
    }

    fn complex_ack(&self, ack_buf: BytesMut) -> Apdu {
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: self.req.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: self.req.service_choice,
            service_ack: ack_buf.freeze(),
        })
    }

    pub(super) async fn write_property<T: TransportPort + 'static>(
        &self,
        services: &RequestServices<T>,
        effects: &mut MutationEffects,
        audit: &mut audit_reporter::WriteAudit<'_, T>,
    ) -> Apdu {
        let (db, cov_table) = (&services.db, &services.cov_table);
        let MutationEffects {
            settle,
            coarse_cov_oids,
            life_safety_cov_changes,
            staging_plans,
            command_runs,
            ..
        } = effects;
        if let Err(error) = self
            .authorize(|| {
                WritePropertyRequest::decode(&self.req.service_request)
                    .map(MutationTarget::WriteProperty)
            })
            .await
        {
            return self.error(&error);
        }
        // A write the object saves first saves here, without the guard.
        let staged = durable_writes::stage(
            db,
            durable_writes::DurableTarget::write_property(&self.req.service_request),
        )
        .await;
        let database = db;
        let (result, exact_changes, plans, schedule_cov) = {
            let mut db = db.write().await;
            let snapshots = bacnet_device_core::cov::life_safety::LifeSafetyCovSnapshots::capture_write_property(
                &db,
                &self.req.service_request,
            );
            let source = audit.write_source();
            let result = bacnet_device_core::write::write_property(
                &mut db,
                &self.req.service_request,
                Some(audit),
                Some(&source),
                self.command_origin,
            );
            staged.release(&mut db);
            // Post-write work follows a change. A NULL the property left as
            // it was (`bacnet_device_core::write::relinquish`) is acknowledged with none.
            let written = match &result {
                Ok((oid, bacnet_device_core::write::Applied::Written)) => Some(*oid),
                Ok((_, bacnet_device_core::write::Applied::Unchanged)) | Err(_) => None,
            };
            let changes = written
                .map(|oid| snapshots.changes(&db, std::slice::from_ref(&oid)))
                .unwrap_or_default();
            let plans = written.map_or_else(Vec::new, |oid| {
                BACnetServer::<T>::take_staging_plans(&mut db, std::slice::from_ref(&oid))
            });
            if let Some(oid) = written {
                command_runs.extend(TakenRuns::take(
                    database,
                    &mut db,
                    std::slice::from_ref(&oid),
                ));
                // A Life Safety object is captured exactly as written, ahead
                // of its transition, as the core's request path captures each
                // attempt; neither host's capture after a transition selects
                // a Life Safety reference.
                let life_safety = bacnet_device_core::cov::life_safety::is_life_safety_object(oid);
                if life_safety {
                    let capture = cov_table.read().await.timed_capture_exact(&changes);
                    capture.run(&db);
                }
                // The transition the write causes commits under its guard
                // too, ahead of a written Schedule's pass (a Schedule has no
                // intrinsic reporting). WPM evaluates after the pass, since it
                // can write a Schedule and one of its targets. The pass
                // evaluates a Schedule whose Reliability it changes; here an
                // application's own reporting Schedule-type object whose
                // Present_Value alone the pass changes gets that transition
                // at the next sweep.
                commit_written_transition(|| services.event_delivery(), &mut db, oid, settle);
                // Any other object is captured once the transition has
                // committed: one row for the write and its transition (#2535).
                if !life_safety {
                    let capture = cov_table.read().await.timed_capture(oid);
                    capture.run(&db);
                }
            }
            let written_slice = written.as_slice();
            let schedule_cov = reevaluate_schedules(services, &mut db, written_slice, settle).await;
            (result.map(|_| written), changes, plans, schedule_cov)
        };
        staging_plans.extend(plans);
        let response = match result {
            Ok(Some(oid)) => {
                if bacnet_device_core::cov::life_safety::is_life_safety_object(oid) {
                    *life_safety_cov_changes = exact_changes;
                } else {
                    coarse_cov_oids.push(oid);
                }
                self.simple_ack()
            }
            Ok(None) => self.simple_ack(),
            Err(e) => self.error(&e),
        };
        // Targets a written Schedule commanded on re-evaluation.
        schedule_cov.merge_into(coarse_cov_oids, life_safety_cov_changes, command_runs);
        response
    }

    pub(super) async fn write_property_multiple<T: TransportPort + 'static>(
        &self,
        services: &RequestServices<T>,
        effects: &mut MutationEffects,
        audit: &mut audit_reporter::WriteAudit<'_, T>,
    ) -> Apdu {
        let (db, cov_table) = (&services.db, &services.cov_table);
        let MutationEffects {
            settle,
            coarse_cov_oids,
            life_safety_cov_changes,
            staging_plans,
            command_runs,
            timed_revisits,
        } = effects;
        // A policy decides every attempt here, with no database guard held,
        // and the handler takes those decisions as it reaches each attempt
        // under the write lock (#2032). Attempts the objects save first are
        // staged here too, without the guard, once decided (#1321).
        let mut ahead = wpm_ahead::DecidedAhead::default();
        let targets = ahead.targets(self, db).await;
        let staged = durable_writes::stage(db, targets).await;
        let database = db;
        let (outcome, exact_changes, plans, schedule_cov) = {
            let mut db = db.write().await;
            // Lock order: database, then a short table read. Each attempt is
            // captured as it commits, under this guard (#856).
            let capture = cov_table.read().await.timed_capture_all();
            // Nothing is asked under this guard: `take` answers when a policy
            // was asked ahead, and the mode answers otherwise.
            let authorize = |attempt: &bacnet_services::wpm::WritePropertyAttempt| {
                let decision = ahead
                    .take(attempt)
                    .or_else(|| self.decide_without_policy())
                    .unwrap_or(MutationDecision::Deny);
                self.record(decision)
            };
            let source = audit.write_source();
            let mut observer = capture.for_write(Some(audit));
            let outcome = write::write_property_multiple(
                &mut db,
                &self.req.service_request,
                Some(&authorize),
                Some(&mut observer),
                Some(&source),
                self.command_origin,
            );
            staged.release(&mut db);
            let committed_oids = match &outcome {
                WpmOutcome::Success { committed_oids }
                | WpmOutcome::Error { committed_oids, .. } => committed_oids.as_slice(),
                WpmOutcome::Reject { .. } => &[],
            };
            let changes = observer.life_safety_changes(&db, committed_oids);
            timed_revisits.extend_from_slice(observer.life_safety_queued());
            let plans = BACnetServer::<T>::take_staging_plans(&mut db, committed_oids);
            command_runs.extend(TakenRuns::take(database, &mut db, committed_oids));
            let schedule_cov =
                reevaluate_schedules(services, &mut db, committed_oids, settle).await;
            // Each written object's transition commits under this guard too,
            // in the order the objects were written. Its attempts were
            // captured as they committed: an object whose transition commits
            // is captured again, in place of the newest row of each reference
            // its attempts queued one for, so the write and its transition
            // are one row (#2535). A snapshot fanout that drained such a row
            // in between (it takes no database guard) leaves this capture to
            // follow it, as two rows.
            for oid in committed_oids {
                // The closure is a temporary of the condition, gone before
                // the capture below awaits the table.
                if commit_written_transition(|| services.event_delivery(), &mut db, *oid, settle) {
                    let capture = cov_table.read().await.timed_capture(*oid);
                    capture.run_replacing(&db, observer.queued(*oid));
                }
            }
            coarse_cov_oids.extend(
                committed_oids.iter().copied().filter(|oid| {
                    !bacnet_device_core::cov::life_safety::is_life_safety_object(*oid)
                }),
            );
            (outcome, changes, plans, schedule_cov)
        };
        staging_plans.extend(plans);
        let response = match outcome {
            WpmOutcome::Success { .. } => self.simple_ack(),
            WpmOutcome::Error {
                error,
                first_failed_write_attempt,
                ..
            } => {
                let (error_class, error_code) = bacnet_device_core::reply::error_fields(&error);
                Apdu::Error(
                    bacnet_services::wpm::WritePropertyMultipleError {
                        error_class,
                        error_code,
                        first_failed_write_attempt,
                    }
                    .to_error_pdu(self.req.invoke_id),
                )
            }
            WpmOutcome::Reject { reason } => Apdu::Reject(RejectPdu {
                invoke_id: self.req.invoke_id,
                reject_reason: reason,
            }),
        };
        *life_safety_cov_changes = exact_changes;
        // Targets a written Schedule commanded on re-evaluation.
        schedule_cov.merge_into(coarse_cov_oids, life_safety_cov_changes, command_runs);
        response
    }

    pub(super) async fn subscribe_cov(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        if let Err(error) = self
            .authorize(|| {
                SubscribeCOVRequest::decode(&self.req.service_request)
                    .map(MutationTarget::SubscribeCov)
            })
            .await
        {
            return self.error(&error);
        }
        let db = db.read().await;
        let mut table = cov_table.write().await;
        match handlers::handle_subscribe_cov_with_initial_endpoint(
            &mut table,
            &db,
            self.source_mac,
            self.source_network,
            &self.req.service_request,
        ) {
            Ok(subscriptions) => {
                initial_cov_notifications.extend(
                    subscriptions
                        .into_iter()
                        .map(Box::new)
                        .map(InitialCovNotification::Single),
                );
                self.simple_ack()
            }
            Err(e) => self.error(&e),
        }
    }

    pub(super) async fn subscribe_cov_property(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        if let Err(error) = self
            .authorize(|| {
                SubscribeCOVPropertyRequest::decode(&self.req.service_request)
                    .map(MutationTarget::SubscribeCovProperty)
            })
            .await
        {
            return self.error(&error);
        }
        let db = db.read().await;
        let mut table = cov_table.write().await;
        match handlers::handle_subscribe_cov_property_with_initial_endpoint(
            &mut table,
            &db,
            self.source_mac,
            self.source_network,
            &self.req.service_request,
        ) {
            Ok(subscriptions) => {
                initial_cov_notifications.extend(
                    subscriptions
                        .into_iter()
                        .map(Box::new)
                        .map(InitialCovNotification::Single),
                );
                self.simple_ack()
            }
            Err(e) => self.error(&e),
        }
    }

    pub(super) async fn atomic_write_file<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        audit: &mut super::super::audit_reporter::WriteAudit<'_, T>,
    ) -> Apdu {
        if let Err(error) = self
            .authorize(|| {
                AtomicWriteFileRequest::decode(&self.req.service_request)
                    .map(MutationTarget::AtomicWriteFile)
            })
            .await
        {
            return self.error(&error);
        }
        let mut db = db.write().await;
        BACnetServer::<T>::atomic_write_file_response(
            &mut db,
            self.req.invoke_id,
            &self.req.service_request,
            self.config.atomic_write_file_budget,
            audit,
        )
    }

    /// AddListElement (`remove` false) or RemoveListElement. A Schedule whose
    /// references changed runs its pass at once under the same guard, as
    /// after a WriteProperty, so an added target gets the current value and a
    /// removed one is relinquished (#1121). The edited object joins the
    /// per-write event evaluation, so an Alarm_Values edit that puts the
    /// watched value in or out of alarm starts its transition at once rather
    /// than at the next periodic tick. It also gets the COV fanout a
    /// WriteProperty would give it, since an edit can move a reported value:
    /// masking an Access Door's Door_Alarm_State returns it to NORMAL
    /// (#1149). Life Safety objects keep their exact-change path, which a
    /// list edit doesn't feed.
    pub(super) async fn list_element<T: TransportPort + 'static>(
        &self,
        services: &RequestServices<T>,
        effects: &mut MutationEffects,
        audit: &mut super::super::audit_reporter::WriteAudit<'_, T>,
        remove: bool,
    ) -> Apdu {
        let (db, cov_table) = (&services.db, &services.cov_table);
        if let Err(error) = self
            .authorize(|| {
                ListElementRequest::decode(&self.req.service_request).map(if remove {
                    MutationTarget::RemoveListElement
                } else {
                    MutationTarget::AddListElement
                })
            })
            .await
        {
            return self.error(&error);
        }
        // A list the object saves first saves here, without the guard.
        let staged = durable_writes::stage(
            db,
            durable_writes::DurableTarget::list_element(&self.req.service_request, remove),
        )
        .await;
        let (result, written, schedule_cov) = {
            let mut db = db.write().await;
            let (result, written) = match handlers::handle_list_element_observed(
                &mut db,
                &self.req.service_request,
                remove,
                |db, request, current| audit.before_list(db, request, current),
            ) {
                Ok(oid) => (Ok(()), Some(oid)),
                Err(error) => (Err(error), None),
            };
            staged.release(&mut db);
            audit.lifecycle_completed(&mut db, &result);
            // Lock order: database, then a short table read, as after a
            // WriteProperty: timestamped references capture the edit now.
            if let Some(oid) = written {
                // The edit's transition commits under its guard too, ahead
                // of a written Schedule's pass, as after a WriteProperty (WPM
                // evaluates after it). The pass evaluates a Schedule whose
                // Reliability it changes; an application's own reporting
                // Schedule-type object whose Present_Value alone the pass
                // changes gets that transition at the next sweep. The edit
                // is captured after it (#2535).
                let settle = &mut effects.settle;
                commit_written_transition(|| services.event_delivery(), &mut db, oid, settle);
                let capture = cov_table.read().await.timed_capture(oid);
                capture.run(&db);
            }
            let settle = &mut effects.settle;
            let schedule_cov =
                reevaluate_schedules(services, &mut db, written.as_slice(), settle).await;
            (result, written, schedule_cov)
        };
        // The edited object first, then a Schedule's targets, which the merge
        // adds only when not already there, as after a WriteProperty.
        effects.coarse_cov_oids.extend(
            written
                .filter(|oid| !bacnet_device_core::cov::life_safety::is_life_safety_object(*oid)),
        );
        schedule_cov.merge_into(
            &mut effects.coarse_cov_oids,
            &mut effects.life_safety_cov_changes,
            &mut effects.command_runs,
        );
        match result {
            Ok(()) => self.simple_ack(),
            Err(e) => self.error(&e),
        }
    }

    pub(super) async fn subscribe_cov_property_multiple(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        let decoded = SubscribeCOVPropertyMultipleRequest::decode(&self.req.service_request)
            .map_err(Error::into_request_reject);
        match decoded {
            Err(e) => self.error(&e),
            Ok(request) => {
                if let Err(error) = self
                    .authorize(|| {
                        Ok(MutationTarget::SubscribeCovPropertyMultiple(
                            request.clone(),
                        ))
                    })
                    .await
                {
                    return self.error(&error);
                }
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property_multiple_request_endpoint(
                    &mut table,
                    &db,
                    self.source_mac,
                    self.source_network,
                    Some(self.req.max_apdu_length),
                    request,
                ) {
                    Ok(subscriptions) => {
                        if !subscriptions.is_empty() {
                            initial_cov_notifications
                                .push(InitialCovNotification::Multiple(subscriptions));
                        }
                        self.simple_ack()
                    }
                    // The references kept before the failed one are reported
                    // as an accepted request's would be (Clause 13.16.2).
                    Err(refusal) => {
                        if !refusal.committed.is_empty() {
                            initial_cov_notifications
                                .push(InitialCovNotification::Multiple(refusal.committed));
                        }
                        self.error(&refusal.error)
                    }
                }
            }
        }
    }
}

#[path = "wpm_ahead.rs"]
mod wpm_ahead;

#[cfg(test)]
#[path = "mutation_policy_tests.rs"]
mod policy_tests;
