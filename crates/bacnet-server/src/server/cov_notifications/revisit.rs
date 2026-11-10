//! Fan acknowledged or fenced references out again (#896).
//!
//! A confirmed report advances its baseline only on the Ack, and changes made
//! while it was outstanding were held back. The acknowledged subscription, or
//! every live reference of the acknowledged context, is evaluated again through
//! the usual fanout, so it reports whatever now differs from its baseline
//! (Status_Flags and queued timestamped changes included) and nothing when it
//! is unchanged. References whose outstanding report was fenced by a route
//! change or re-subscription come through here too.
use super::super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::*;
use bacnet_device_core::cov::CovSubscriptionKey;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Evaluate the live references among `keys` against their baselines and
    /// report what changed. Under DCC the evaluation is dropped, as any fanout
    /// is, not deferred: the references report again on their next fanout.
    pub(in crate::server) async fn fire_cov_revisits(
        ctx: &CovNotifyContext<'_, T>,
        keys: &[CovSubscriptionKey],
    ) {
        if ctx.comm_state.initiation_restricted() {
            return;
        }
        // In key order, by monitored object first, whichever order they
        // were queued or came due in. A follow-up report lists its
        // untimestamped items in the order its references are fanned out, and
        // the revisit queue is a hash set: that order, and the order between
        // follow-ups, varied from run to run (#2320).
        let keys: std::collections::BTreeSet<_> = keys.iter().collect();
        let (subs, counters, in_flight_tracker, timed) = {
            let table = ctx.cov_table.read().await;
            (
                keys.iter()
                    .filter_map(|key| table.get_subscription(key))
                    .filter(|sub| table.is_current(sub))
                    .cloned()
                    .collect::<Vec<_>>(),
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                table.timed().clone(),
            )
        };
        if subs.is_empty() {
            return;
        }
        let handles = CovFanoutHandles {
            ctx,
            in_flight_tracker: &in_flight_tracker,
            counters: &counters,
            timed: &timed,
        };
        // One budget per object or context, as a natural event would have.
        // Objects, like contexts, in key order.
        let mut single_by_object: std::collections::BTreeMap<
            ObjectIdentifier,
            Vec<CovSubscriptionSnapshot>,
        > = std::collections::BTreeMap::new();
        let mut multiple = Vec::new();
        for sub in subs {
            if sub.key().multiple_context().is_some() {
                multiple.push(sub);
            } else {
                single_by_object
                    .entry(sub.monitored_object_identifier)
                    .or_default()
                    .push(sub);
            }
        }
        // Contexts in the order of their keys, as every Multiple fanout
        // takes them (#2320).
        let multiple_by_context = bacnet_device_core::cov::multiple_report::by_context(multiple);
        for (oid, subs) in &single_by_object {
            Self::fire_cov_notifications_for_subscriptions(
                &handles,
                oid,
                subs,
                None,
                false,
                &mut EventBudget::new(&ctx.config.cov_policy),
            )
            .await;
        }
        for subs in multiple_by_context.values() {
            Self::fire_cov_notification_multiple_for_subscriptions(
                &handles,
                subs,
                None,
                false,
                &mut EventBudget::new(&ctx.config.cov_policy),
            )
            .await;
        }
    }
}
