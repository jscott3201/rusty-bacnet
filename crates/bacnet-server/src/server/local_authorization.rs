//! Local completed-attempt authorization and its recorded event evidence.

use super::cov_fanout::CovFanout;
use super::local_writes::local_runtime;
use super::*;
use bacnet_objects::access_control::{
    AccessEventReport, AuthorizationEvaluation, AuthorizationOutcome, AuthorizationRequest,
    evaluate_authorization,
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
    /// transactions have no global notification-order guarantee.
    pub async fn authorize_access_local(
        &self,
        request: AuthorizationRequest<'_>,
        tag: u64,
    ) -> Result<LocalAuthorizationResult, Error> {
        let runtime = local_runtime()?;
        self.active_network()?;
        let oid = request.original.point;
        let (mut db, table) = self.authorization_guards().await;
        admit(&db, &oid)?;
        let evaluation = evaluate_authorization(&db, request)?;
        if !matches!(
            evaluation.outcome,
            AuthorizationOutcome::Granted | AuthorizationOutcome::Denied { .. }
        ) {
            return Ok(LocalAuthorizationResult {
                evaluation,
                committed_events: Vec::new(),
            });
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
            .expect("admitted point remains under the same database guard")
            .record_access_event_batch_internal(&reports)?;
        let records = committed.iter().map(|event| event.report.clone()).collect();
        for event in &committed {
            table
                .timed_capture(oid)
                .run_from_snapshot(event.snapshot.as_ref(), event.clock_frame);
        }
        // No await after the atomic install and capture until post-commit work
        // belongs to the server's task set. The COV read guard excludes admission
        // through capture, and the database guard is released before fanout awaits.
        drop(table);
        let fanout = CovFanout::new(&self.local_writer().cov_context(), &self.event_suppressions);
        let routers = Arc::clone(&self.learned_routers);
        let bindings = Arc::clone(&self.device_bindings);
        self.finish_in_task(&runtime, async move {
            drop(db);
            Self::fire_event_notifications_with_bindings(
                &fanout.event_delivery(&routers, &bindings),
                &fanout.cov_table,
                &oid,
            )
            .await;
            for event in committed {
                Self::fire_cov_notifications_from_snapshot(
                    &fanout.notify_context(),
                    &oid,
                    event.snapshot.as_ref(),
                )
                .await;
            }
        })
        .await?;
        Ok(LocalAuthorizationResult {
            evaluation,
            committed_events: records,
        })
    }

    /// Acquire DB before COV without retaining a lock across either queued wait.
    /// A failed try-read performs no evaluation and retains no subscription data.
    async fn authorization_guards(
        &self,
    ) -> (
        OwnedRwLockWriteGuard<ObjectDatabase>,
        OwnedRwLockReadGuard<CovSubscriptionTable>,
    ) {
        loop {
            let db = Arc::clone(&self.db).write_owned().await;
            if let Ok(table) = Arc::clone(&self.cov_table).try_read_owned() {
                return (db, table);
            }
            drop(db);
            drop(self.cov_table.read().await);
            tokio::task::yield_now().await;
        }
    }
}

/// Reject unhealthy or malformed admission before invoking the pure evaluator.
fn admit(db: &ObjectDatabase, oid: &ObjectIdentifier) -> Result<(), Error> {
    let object = db.get(oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;
    if oid.object_type() != ObjectType::ACCESS_POINT {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ));
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
    if !object.supports_access_event_batch_internal() {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ));
    }
    Ok(())
}
