use super::super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::*;

/// Fire exact-delta COV notifications for one Life Safety object.
///
/// Whole-object subscriptions observe only Present_Value/Status_Flags.
/// Property subscriptions observe their property and every actual
/// Status_Flags change. Callers supply committed readback deltas after
/// releasing the object-database write lock.
impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(in crate::server) async fn fire_life_safety_cov_notifications(
        ctx: &CovNotifyContext<'_, T>,
        oid: &ObjectIdentifier,
        changed_properties: &[PropertyIdentifier],
    ) {
        if ctx.comm_state.initiation_restricted() || changed_properties.is_empty() {
            return;
        }
        let change = bacnet_device_core::cov::life_safety::LifeSafetyCovChange {
            object_identifier: *oid,
            changed_properties: changed_properties.to_vec(),
        };
        let (subs, counters, in_flight_tracker, dispatch_turn) = {
            let mut table = ctx.cov_table.write().await;
            (
                table
                    .subscriptions_for(oid)
                    .into_iter()
                    .filter(|sub| change.notifies(sub))
                    .cloned()
                    .collect::<Vec<_>>(),
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                table.next_dispatch_turn(),
            )
        };
        if subs.is_empty() {
            return;
        }
        let handles = CovFanoutHandles {
            ctx,
            in_flight_tracker: &in_flight_tracker,
            counters: &counters,
        };
        let force = change.status_changed();
        Self::fire_cov_by_kind(&handles, dispatch_turn, oid, subs, None, force).await;
    }

    pub(in crate::server) async fn fire_post_write_cov_notifications(
        ctx: &CovNotifyContext<'_, T>,
        coarse_oids: &[ObjectIdentifier],
        exact_changes: &[bacnet_device_core::cov::life_safety::LifeSafetyCovChange],
    ) {
        for oid in coarse_oids {
            Self::fire_cov_notifications(ctx, oid).await;
        }
        for change in exact_changes {
            Self::fire_life_safety_cov_notifications(
                ctx,
                &change.object_identifier,
                &change.changed_properties,
            )
            .await;
        }
    }
}
