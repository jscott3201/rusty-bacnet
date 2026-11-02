//! Executing the target writes a Staging object's stage changes queue
//! (Clause 12.62), one database guard per target, after the guard that
//! committed the change is released.

use super::super::cov_notify_context::CovNotifyContext;
use super::super::*;
use crate::command_lists::TakenRuns;
use bacnet_objects::staging::StagingWritePlan;

impl<T: TransportPort + 'static> BACnetServer<T> {
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
    /// that applies it. Event/COV work runs only after that guard is released.
    /// Each guard first does the work the membership queue holds, so a
    /// removed Staging object's NULLs land before one re-added under its
    /// identifier writes (#2265; the Value_Source check can't tell the two
    /// apart). Returns the Command and Channel runs that work queued, for the
    /// caller to start.
    pub(in crate::server) async fn execute_staging_plans(
        delivery: &EventDelivery<'_, T>,
        cov: &CovNotifyContext<'_, T>,
        plans: Vec<StagingWritePlan>,
    ) -> TakenRuns {
        let db = delivery.db;
        let mut runs = TakenRuns::default();
        for plan in plans {
            let mut all_succeeded = true;
            for target in &plan.writes {
                enum TargetResult {
                    Applied,
                    Failed,
                    Stale,
                }

                let (result, membership) = {
                    let mut database = db.write().await;
                    let membership =
                        crate::membership::settle(db, &mut database, cov.cov_table).await;
                    let current = database
                        .get_mut(&plan.source)
                        .and_then(|source| source.staging_source())
                        .map(|source| source.staging_generation())
                        == Some(plan.generation);
                    let result = if !current {
                        TargetResult::Stale
                    } else {
                        let origin = crate::command_source::resolve_local(
                            &database,
                            crate::LocalCommandSource::Object(plan.source),
                        )
                        .ok();
                        match database.get_mut(&target.object_identifier) {
                            Some(object) => match crate::command_source::write_target(
                                object,
                                PropertyIdentifier::PRESENT_VALUE,
                                None,
                                PropertyValue::Enumerated(u32::from(target.active)),
                                Some(plan.priority),
                                origin.as_ref(),
                            ) {
                                Ok(()) => {
                                    // Timestamped references capture the
                                    // target change under this guard (#856).
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
                if !membership.is_empty() {
                    Self::fire_post_write_cov_notifications(
                        cov,
                        &membership.coarse,
                        &membership.life_safety,
                    )
                    .await;
                    runs.extend(membership.command_runs);
                }

                match result {
                    TargetResult::Stale => break,
                    TargetResult::Failed => all_succeeded = false,
                    TargetResult::Applied => {
                        Self::fire_event_notifications_with_bindings(
                            delivery,
                            cov.cov_table,
                            &target.object_identifier,
                        )
                        .await;
                        Self::fire_cov_notifications(cov, &target.object_identifier).await;
                    }
                }
            }

            let reliability_changed = {
                let mut database = db.write().await;
                let changed = database
                    .get_mut(&plan.source)
                    .and_then(|source| source.staging_source())
                    .is_some_and(|source| {
                        source.complete_staging_write_plan(plan.generation, all_succeeded)
                    });
                if changed {
                    let capture = cov.cov_table.read().await.timed_capture(plan.source);
                    capture.run(&database);
                }
                changed
            };
            if reliability_changed {
                Self::fire_event_notifications_with_bindings(delivery, cov.cov_table, &plan.source)
                    .await;
                Self::fire_cov_notifications(cov, &plan.source).await;
            }
        }
        runs
    }
}
