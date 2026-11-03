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
        let status_changed = changed_properties.contains(&PropertyIdentifier::STATUS_FLAGS);
        let (subs, counters, in_flight_tracker, dispatch_turn) = {
            let mut table = ctx.cov_table.write().await;
            (
                table
                    .subscriptions_for(oid)
                    .into_iter()
                    .filter(|sub| match sub.monitored_property {
                        Some(property) => status_changed || changed_properties.contains(&property),
                        None => {
                            status_changed
                                || changed_properties.contains(&PropertyIdentifier::PRESENT_VALUE)
                        }
                    })
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
        Self::fire_cov_by_kind(&handles, dispatch_turn, oid, subs, None, status_changed).await;
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
