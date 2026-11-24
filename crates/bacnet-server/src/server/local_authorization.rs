//! Local completed-attempt authorization and its recorded event evidence.

use super::cov_fanout::CovFanout;
use super::local_writes::local_runtime;
use super::*;
use bacnet_objects::access_control::{
    AccessEventReport, AccompanimentWitness, AuthorizationEvaluation, AuthorizationOccupancy,
    AuthorizationOccupancyZone, AuthorizationOutcome, AuthorizationRequest,
    CompanionAuthenticationAttestation, evaluate_authorization,
};
use bacnet_types::enums::Reliability;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

/// Complete evaluation and the event records installed by one local attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalAuthorizationResult {
    /// The full evaluator result, including diagnostic and intermediate intents.
    pub evaluation: AuthorizationEvaluation,
    /// Records actually installed, in intent order, with their assigned timestamps.
    /// Empty for pending, unresolved, and no-authorization evaluations.
    pub committed_events: Vec<AccessEventReport>,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Evaluate an authenticated local presentation and record a terminal result.
    ///
    /// A grant or denial installs all its event intents in order; pending,
    /// unresolved and no-authorization results publish nothing, including their
    /// intermediate intents. The full request supplies every evaluation policy
    /// and operational fact. The caller owns retries, tag progression and replay
    /// handling: repeating a terminal attempt is a new publication, even when its
    /// tag is unchanged. Evaluation and event installation share one DB guard.
    ///
    /// Each event names its associated credential (the companion for BY) and is
    /// stamped by the point. No physical factor is supplied, so the absent-factor
    /// UNDEFINED representation applies (Clauses 12.31.27–31). Equal/backward
    /// clocks advance the event trigger, and an exhausted timestamp representation
    /// refuses the entire batch. This route performs no physical door action.
    ///
    /// Runtime, stopped-network, unknown/wrong object, unsupported batch, malformed
    /// admission and out-of-service/fault errors leave state unchanged. Contention
    /// retries release all guards before waiting and observe fresh state; progress
    /// is cooperative once contention quiesces, not a starvation/latency guarantee.
    ///
    /// A committed attempt retains snapshots for ordered COV fanout through the
    /// existing notification lifecycle. Caller cancellation after commit does not
    /// discard owned work. DCC, budgets, send failures and shutdown still apply;
    /// success is not remote acknowledgment or durable delivery. Independent
    /// transactions have no global notification-order guarantee. Neither
    /// guard is waited for in the caller's task, and a call dropped part way
    /// is as [`write_local`](Self::write_local)'s Cancellation section describes
    /// (#2772): one dropped after its task took both guards may still
    /// record its events.
    pub async fn authorize_access_local(
        &self,
        request: AuthorizationRequest<'_>,
        tag: u64,
    ) -> Result<LocalAuthorizationResult, Error> {
        let runtime = local_runtime()?;
        self.active_network()?;
        let oid = request.original.point;
        let request = OwnedRequest::new(request);
        let fanout = CovFanout::new(
            &self.local_writer().cov_context(),
            &self.event_suppressions,
            &self.learned_routers,
            &self.device_bindings,
        );
        let tasks = Arc::clone(&self.request_tasks);
        let guards = authorization_guards(Arc::clone(&self.db), Arc::clone(&self.cov_table));
        // Synchronous under both guards; the post-commit work goes to a task
        // of its own with the database guard.
        let attempt = move |(mut db, table): Guards| -> Result<_, Error> {
            admit(&mut db, &oid)?;
            let evaluation = evaluate_authorization(&db, request.borrow())?;
            if !matches!(
                evaluation.outcome,
                AuthorizationOutcome::Granted | AuthorizationOutcome::Denied { .. }
            ) {
                let result = LocalAuthorizationResult {
                    evaluation,
                    committed_events: Vec::new(),
                };
                return Ok((result, None));
            }
            let reports: Vec<_> = evaluation
                .events
                .iter()
                .map(|intent| AccessEventReport {
                    credential: Some(intent.presentation.credential.into()),
                    ..AccessEventReport::new(intent.access_event, tag)
                })
                .collect();
            let committed = db
                .get_mut(&oid)
                .and_then(|object| object.access_events())
                .expect("admitted point remains under the same database guard")
                .record_access_event_batch(&reports)?;
            let records = committed.iter().map(|event| event.report.clone()).collect();
            // The references any event queued a row for: a transition of the
            // point is captured in place of each one's newest row (#2535).
            let mut queued = Vec::new();
            for event in &committed {
                let keys = table
                    .timed_capture(oid)
                    .run_from_snapshot(event.snapshot.as_ref(), event.clock_frame);
                for key in keys {
                    if !queued.contains(&key) {
                        queued.push(key);
                    }
                }
            }
            // No await after the atomic install and capture until post-commit
            // work belongs to the server's task set. The COV read guard
            // excludes admission through capture, and the database guard is
            // released before fanout awaits.
            drop(table);
            let finish = tasks.finish_in_task(&runtime, async move {
                // The point's transition commits under the attempt's guard
                // (#2530). The built-in Access Point reports no intrinsic
                // events; an application's own point that does is evaluated
                // here.
                let mut db = db;
                let mut settle = super::event_send::Settle::default();
                let delivery = fanout.event_delivery();
                // Its notification joins its recipients' lanes under the
                // guard (#2537). No test drives this replacement: the
                // built-in Access Point reports no intrinsic events, and a
                // test point that did would need its own detector, commit and
                // event properties.
                let transitioned = super::event_notifications::commit_written_transition(
                    || delivery,
                    &mut db,
                    oid,
                    &mut settle,
                );
                if transitioned {
                    let capture = fanout.cov_table.read().await.timed_capture(oid);
                    capture.run_replacing(&db, &queued);
                }
                drop(db);
                // The COV fanout waits for it, capped (#889).
                let timeout_ms = delivery.retry_timeout_ms;
                let transactions = delivery.notification_transactions;
                super::event_send::settled(settle.into_wait(), timeout_ms, transactions).await;
                for event in committed {
                    Self::fire_cov_notifications_from_snapshot(
                        &fanout.notify_context(),
                        &oid,
                        event.snapshot.as_ref(),
                    )
                    .await;
                }
            });
            let result = LocalAuthorizationResult {
                evaluation,
                committed_events: records,
            };
            Ok((result, Some(finish)))
        };
        // Neither guard is waited for in the caller's task (#2772).
        let (result, finish) = self
            .request_tasks
            .for_caller(guards, attempt)
            .await
            // `None`: no change; its runtime dropped the task before it took its locks.
            .unwrap_or_else(|| Err(crate::server::broadcaster::stopped()))?;
        if let Some(finish) = finish {
            finish.await?;
        }
        Ok(result)
    }
}

/// The database's write guard and the COV table's read guard an attempt
/// evaluates and records under.
type Guards = (
    OwnedRwLockWriteGuard<ObjectDatabase>,
    OwnedRwLockReadGuard<CovSubscriptionTable>,
);

/// Acquire DB before COV without retaining a lock across either queued wait.
/// A failed try-read performs no evaluation and retains no subscription data.
async fn authorization_guards(
    db: Arc<RwLock<ObjectDatabase>>,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
) -> Guards {
    loop {
        let guard = Arc::clone(&db).write_owned().await;
        if let Ok(table) = Arc::clone(&cov_table).try_read_owned() {
            return (guard, table);
        }
        drop(guard);
        drop(cov_table.read().await);
        tokio::task::yield_now().await;
    }
}

/// An [`AuthorizationRequest`] that owns the lists it borrows, so a task of
/// the server's can evaluate it when the database is busy (#2772).
struct OwnedRequest {
    /// The request, its lists left empty.
    request: AuthorizationRequest<'static>,
    /// The zones of [`AuthorizationOccupancy::Zones`], when it names them.
    zones: Option<Vec<AuthorizationOccupancyZone>>,
    witnesses: Vec<AccompanimentWitness>,
    attestations: Vec<CompanionAuthenticationAttestation>,
}

impl OwnedRequest {
    fn new(request: AuthorizationRequest<'_>) -> Self {
        let (occupancy, zones) = match request.occupancy {
            AuthorizationOccupancy::NotApplicable => (AuthorizationOccupancy::NotApplicable, None),
            AuthorizationOccupancy::Unknown => (AuthorizationOccupancy::Unknown, None),
            AuthorizationOccupancy::Zones(zones) => {
                (AuthorizationOccupancy::Zones(&[]), Some(zones.to_vec()))
            }
        };
        Self {
            request: AuthorizationRequest {
                original: request.original,
                wall_time: request.wall_time,
                now: request.now,
                order: request.order,
                disable_reason_priority: request.disable_reason_priority,
                rights_policy: request.rights_policy,
                fallback_threat_level: request.fallback_threat_level,
                fallback_lockout: request.fallback_lockout,
                occupancy,
                passback: request.passback,
                accompaniment_witnesses: &[],
                accompaniment_timing: request.accompaniment_timing,
                max_accompaniment_links: request.max_accompaniment_links,
                companion_authentication: &[],
                external_phase: request.external_phase,
            },
            zones,
            witnesses: request.accompaniment_witnesses.to_vec(),
            attestations: request.companion_authentication.to_vec(),
        }
    }

    /// The request as the caller made it.
    fn borrow(&self) -> AuthorizationRequest<'_> {
        AuthorizationRequest {
            occupancy: match &self.zones {
                Some(zones) => AuthorizationOccupancy::Zones(zones),
                None => self.request.occupancy,
            },
            accompaniment_witnesses: &self.witnesses,
            companion_authentication: &self.attestations,
            ..self.request
        }
    }
}

/// Reject unhealthy or malformed admission before invoking the pure evaluator.
fn admit(db: &mut ObjectDatabase, oid: &ObjectIdentifier) -> Result<(), Error> {
    let object = db.get(oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;
    if oid.object_type() != ObjectType::ACCESS_POINT {
        return Err(bacnet_objects::traits::not_supported());
    }
    let healthy = match (
        object.read_property(PropertyIdentifier::OUT_OF_SERVICE, None)?,
        object.read_property(PropertyIdentifier::RELIABILITY, None)?,
    ) {
        (PropertyValue::Boolean(out_of_service), PropertyValue::Enumerated(reliability)) => {
            !out_of_service && reliability == Reliability::NO_FAULT_DETECTED.to_raw()
        }
        _ => {
            return Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
            ));
        }
    };
    if !healthy {
        return Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ));
    }
    // The point takes the batch only through its AccessEvents capability.
    let batches = db.get_mut(oid).and_then(|object| object.access_events());
    if batches.is_none() {
        return Err(bacnet_objects::traits::not_supported());
    }
    Ok(())
}
