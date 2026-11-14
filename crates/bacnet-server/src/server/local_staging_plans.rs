//! Executing the target writes a Staging object's stage changes queue
//! (Clause 12.62), one database guard per target, after the guard that
//! committed the change is released.

use super::super::cov_notify_context::CovNotifyContext;
use super::super::*;
use crate::command_lists::TakenRuns;
use crate::committed_cov::CommittedCov;
use crate::server::event_notifications::{commit_written_transition, committing};
use crate::server::event_send::{Settle, settled as settled_lanes};
use bacnet_device_core::cov::life_safety::LifeSafetyCovChange;
use bacnet_device_core::write::local;
use bacnet_objects::staging::StagingWritePlan;

/// A peer request's COV fanout, made after its response, that its Staging
/// plans' targets join: its whole-object and Life Safety changes.
pub(in crate::server) type RequestCov<'a> = (
    &'a mut Vec<ObjectIdentifier>,
    &'a mut Vec<LifeSafetyCovChange>,
);

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Wait, capped, for the event notifications a plan's write pushed, so
    /// they go before its COV fanout (#889, #2537). Boxed, and only when it
    /// pushed any.
    async fn settled_for_staging(delivery: &EventDelivery<'_, T>, settle: Settle) {
        if let Some(settled) = settle.into_wait() {
            let wait = settled_lanes(
                Some(settled),
                delivery.retry_timeout_ms,
                delivery.notification_transactions,
            );
            Box::pin(wait).await;
        }
    }

    /// A peer request's Staging plans ([`Self::execute_staging_plans`]),
    /// their targets counted in the request's `settle`, which comes back,
    /// and their objects added to its COV fanout, `cov`.
    pub(in crate::server) async fn execute_request_staging_plans(
        services: &super::request_services::RequestServices<T>,
        plans: Vec<StagingWritePlan>,
        mut settle: Settle,
        cov: RequestCov<'_>,
    ) -> (TakenRuns, Settle) {
        let (delivery, cov_ctx) = (services.event_delivery(), services.cov_context());
        let request = Some((&mut settle, cov));
        let runs = Self::execute_staging_plans(&delivery, &cov_ctx, plans, request).await;
        (runs, settle)
    }

    /// The work the membership queue holds, and the NULLs the plan's source
    /// owes, under `database`, the guard a plan's write takes
    /// ([`crate::membership::settle_for_staging`]); the objects they change
    /// commit their transitions there too (#2535), counted in `settle`.
    async fn membership_for_staging(
        delivery: &EventDelivery<'_, T>,
        cov: &CovNotifyContext<'_, T>,
        database: &mut ObjectDatabase,
        source: ObjectIdentifier,
        settle: &mut Settle,
    ) -> CommittedCov {
        let mut transition = committing(|| *delivery, settle);
        let (db, table) = (delivery.db, cov.cov_table);
        crate::membership::settle_for_staging(db, database, table, source, &mut transition).await
    }

    /// `owed`'s COV fanout, made once its guard's notifications are handed
    /// off: with `request`, its objects join the peer request's fanout, made
    /// after the response; otherwise here, after the caller's wait. Returns
    /// the runs it queued.
    async fn membership_fanout(
        cov: &CovNotifyContext<'_, T>,
        owed: CommittedCov,
        request: Option<&mut RequestCov<'_>>,
    ) -> TakenRuns {
        let mut runs = TakenRuns::default();
        match request {
            Some((coarse, life_safety)) => owed.merge_into(coarse, life_safety, &mut runs),
            None => {
                if !owed.is_empty() {
                    let (coarse, life_safety) = (&owed.coarse, &owed.life_safety);
                    Self::fire_post_write_cov_notifications(cov, coarse, life_safety).await;
                }
                runs = owed.command_runs;
            }
        }
        runs
    }

    pub(in crate::server) async fn execute_initial_staging_plans(&self) {
        let staging_oids = {
            let database = self.db.read().await;
            database.find_by_type(ObjectType::STAGING)
        };
        let staging_plans = {
            let mut database = self.db.write().await;
            Self::take_staging_plans(&mut database, &staging_oids)
        };
        let runs = Self::execute_staging_plans(
            &self.local_event_delivery(),
            &self.local_cov_context(),
            staging_plans,
            None,
        )
        .await;
        super::command_runs::CommandRunner::for_server(self).start(runs);
    }

    pub(in crate::server) fn take_staging_plans(
        db: &mut ObjectDatabase,
        oids: &[ObjectIdentifier],
    ) -> Vec<StagingWritePlan> {
        oids.iter()
            .filter_map(|oid| {
                db.get_mut(oid)
                    .and_then(|object| object.staging_source())
                    .and_then(|source| source.take_staging_write_plan())
            })
            .collect()
    }

    /// Execute local Staging targets one mutation guard at a time.
    ///
    /// Every target mutation is generation-checked in the same database guard
    /// that applies it, and the intrinsic transition it causes commits there
    /// too (#2530). Event and COV sends run only after that guard is released.
    /// Each guard first does the work the membership queue holds, so a
    /// removed Staging object's NULLs land before one re-added under its
    /// identifier writes (#2265; the Value_Source check can't tell the two
    /// apart), then writes the NULLs the plan's source owes for a change of
    /// its targets or priority (#2289), stale plan or not, so each lands
    /// before the source writes that slot again. A plan with no targets
    /// takes one guard for that alone. Returns the Command and Channel runs
    /// that work queued, for the caller to start.
    ///
    /// Each written target's COV fanout waits, capped, for the event
    /// notifications its transition pushed (#889, #2537), and so does that of
    /// the objects the membership work changed, whose transitions commit
    /// under the same guard (#2535). With `request`, a peer request's own
    /// settle handle and COV lists, those notifications count in the
    /// request's handle and the objects join the request's COV fanout, which
    /// the request makes after its response, so the response never waits on
    /// a lane; start-up and local-write plans, which have no response, wait
    /// here.
    pub(in crate::server) async fn execute_staging_plans(
        delivery: &EventDelivery<'_, T>,
        cov: &CovNotifyContext<'_, T>,
        plans: Vec<StagingWritePlan>,
        mut request: Option<(&mut Settle, RequestCov<'_>)>,
    ) -> TakenRuns {
        let db = delivery.db;
        let mut runs = TakenRuns::default();
        for plan in plans {
            if plan.writes.is_empty() {
                let mut settle = Settle::default();
                let owed = {
                    let mut database = db.write().await;
                    let settle = match request.as_mut() {
                        Some((request, _)) => &mut **request,
                        None => &mut settle,
                    };
                    let source = plan.source;
                    Self::membership_for_staging(delivery, cov, &mut database, source, settle).await
                };
                if request.is_none() && !owed.is_empty() {
                    Self::settled_for_staging(delivery, settle).await;
                }
                let request_cov = request.as_mut().map(|(_, cov)| cov);
                runs.extend(Self::membership_fanout(cov, owed, request_cov).await);
            }
            let mut all_succeeded = true;
            for target in &plan.writes {
                enum TargetResult {
                    Applied,
                    Failed,
                    Stale,
                }

                // Its transition, committed with it (#2530), still to send.
                let mut settle = Settle::default();

                let (result, membership) = {
                    let mut database = db.write().await;
                    let membership = {
                        let settle = match request.as_mut() {
                            Some((request, _)) => &mut **request,
                            None => &mut settle,
                        };
                        let source = plan.source;
                        Self::membership_for_staging(delivery, cov, &mut database, source, settle)
                            .await
                    };
                    let current = database
                        .get_mut(&plan.source)
                        .and_then(|source| source.staging_source())
                        .map(|source| source.staging_generation())
                        == Some(plan.generation);
                    let result = if !current {
                        TargetResult::Stale
                    } else {
                        let origin = local::resolve_source(
                            &database,
                            local::LocalCommandSource::Object(plan.source),
                        )
                        .ok();
                        match database.get_mut(&target.object_identifier) {
                            Some(object) => match local::write_as(
                                object,
                                PropertyIdentifier::PRESENT_VALUE,
                                None,
                                PropertyValue::Enumerated(u32::from(target.active)),
                                Some(plan.priority),
                                origin.as_ref(),
                            ) {
                                Ok(()) => {
                                    let settle = match request.as_mut() {
                                        Some((request, _)) => &mut **request,
                                        None => &mut settle,
                                    };
                                    commit_written_transition(
                                        || *delivery,
                                        &mut database,
                                        target.object_identifier,
                                        settle,
                                    );
                                    // Timestamped references capture the
                                    // target change under this guard (#856),
                                    // after its transition: one row (#2535).
                                    let capture = cov
                                        .cov_table
                                        .read()
                                        .await
                                        .timed_capture(target.object_identifier);
                                    capture.run(&database);
                                    TargetResult::Applied
                                }
                                Err(_) => TargetResult::Failed,
                            },
                            None => TargetResult::Failed,
                        }
                    };
                    (result, membership)
                };
                // The membership work's fanout, then the target's, both
                // behind the guard's event notifications (#889): here, one
                // wait before the first of them.
                let applied = matches!(result, TargetResult::Applied);
                if request.is_none() && (applied || !membership.is_empty()) {
                    Self::settled_for_staging(delivery, settle).await;
                }
                let request_cov = request.as_mut().map(|(_, cov)| cov);
                runs.extend(Self::membership_fanout(cov, membership, request_cov).await);

                match result {
                    TargetResult::Stale => break,
                    TargetResult::Failed => all_succeeded = false,
                    TargetResult::Applied => match request.as_mut() {
                        Some((_, (coarse, _))) => coarse.push(target.object_identifier),
                        None => Self::fire_cov_notifications(cov, &target.object_identifier).await,
                    },
                }
            }

            // The source's Reliability and the transition it causes commit
            // under one guard (#2530). The built-in Staging object reports no
            // intrinsic events, so this evaluates only an application's own
            // source, which `StagingSource` lets any object be.
            let mut settle = Settle::default();
            let reliability_changed = {
                let mut database = db.write().await;
                let changed = database
                    .get_mut(&plan.source)
                    .and_then(|source| source.staging_source())
                    .is_some_and(|source| {
                        source.complete_staging_write_plan(plan.generation, all_succeeded)
                    });
                if changed {
                    // No test drives a transition here: the built-in Staging
                    // object has no event reporting, so only an application's
                    // own source could commit one.
                    let settle = match request.as_mut() {
                        Some((request, _)) => &mut **request,
                        None => &mut settle,
                    };
                    commit_written_transition(|| *delivery, &mut database, plan.source, settle);
                    let capture = cov.cov_table.read().await.timed_capture(plan.source);
                    capture.run(&database);
                }
                changed
            };
            if reliability_changed {
                match request.as_mut() {
                    Some((_, (coarse, _))) => coarse.push(plan.source),
                    None => {
                        Self::settled_for_staging(delivery, settle).await;
                        Self::fire_cov_notifications(cov, &plan.source).await;
                    }
                }
            }
        }
        runs
    }
}
