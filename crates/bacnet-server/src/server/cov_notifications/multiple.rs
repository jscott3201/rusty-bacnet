use super::super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::confirmed::ConfirmedReport;
use super::multiple_chunks::{request_limit, split, untimed_octets, Envelope, Part, ReportContent};
use super::multiple_items::{Coordinate, FieldStamp};
use super::*;
use crate::cov::multiple_reads::MultipleReads;
use crate::cov::timed::{FieldTiming, SendTurn, TimedChange, TimedClaim};
use std::collections::HashSet;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Fire the initial COVNotificationMultiple for a newly accepted
    /// SubscribeCOVPropertyMultiple request.
    pub(in crate::server) async fn fire_initial_cov_notification_multiple(
        ctx: &CovNotifyContext<'_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
    ) {
        let (counters, in_flight_tracker, subscriptions) = {
            let table = ctx.cov_table.read().await;
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                subscriptions
                    .iter()
                    .filter(|sub| table.is_current(sub))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        let mut budget = EventBudget::new(&ctx.config.cov_policy);
        Self::fire_cov_notification_multiple_for_subscriptions(
            &CovFanoutHandles {
                ctx,
                in_flight_tracker: &in_flight_tracker,
                counters: &counters,
            },
            &subscriptions,
            None,
            true,
            &mut budget,
        )
        .await;
    }

    pub(in crate::server) async fn fire_cov_notification_multiple_for_subscriptions(
        handles: &CovFanoutHandles<'_, '_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        force: bool,
        budget: &mut EventBudget,
    ) {
        if handles.ctx.comm_state.initiation_restricted() || subscriptions.is_empty() {
            return;
        }

        if budget.is_exhausted() {
            return;
        }

        let mut grouped: HashMap<crate::cov::MultipleContextKey, Vec<CovSubscriptionSnapshot>> =
            HashMap::new();

        for sub in subscriptions {
            grouped
                .entry(
                    sub.key()
                        .multiple_context()
                        .expect("Multiple snapshot")
                        .clone(),
                )
                .or_default()
                .push(sub.clone());
        }

        for subs in grouped.values() {
            Self::send_cov_notification_multiple(handles, subs, snapshot, force, budget).await;
        }
    }

    async fn send_cov_notification_multiple(
        handles: &CovFanoutHandles<'_, '_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        force: bool,
        budget: &mut EventBudget,
    ) {
        let &CovFanoutHandles {
            ctx:
                &CovNotifyContext {
                    db,
                    network,
                    cov_table,
                    config,
                    ..
                },
            counters,
            ..
        } = handles;
        if subscriptions.is_empty() {
            return;
        }

        // The context's send turn, if it is unconfirmed (#986, #1038). Declared
        // before the claim so that, on any early return, the claim's changes
        // are back in their queue before the turn hands the context to a
        // follow-up.
        let mut turn: Option<SendTurn> = None;
        // Timestamped changes drained for this notification; dropping the claim
        // without commit (any early return, failed send) requeues them.
        let mut claim: Option<TimedClaim> = None;
        let (parts, last_notified, representative) = {
            // One DB borrow, released before any send, supplies the Device
            // identity, the clock sample for any current-state fallback and
            // every value. Producers capture under the database write guard,
            // so while this read guard is held no capture can move a field
            // record past the values read here. A producer snapshot carries its
            // own captured changes, so that path takes no clock: it neither
            // adopts a fallback change nor times an uncaptured field (#987).
            let (device_oid, clock_frame, db) = if snapshot.is_none() {
                let db = db.read().await;
                let clock_frame = subscriptions
                    .iter()
                    .any(|sub| sub.timestamped)
                    .then(|| db.clock_frame())
                    .flatten()
                    .filter(|frame| frame.is_valid_actual_datetime());
                (db.selected_device(), clock_frame, Some(db))
            } else {
                (db.read().await.selected_device(), None, None)
            };
            let device_oid =
                device_oid.unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());
            let object_of = |sub: &CovSubscriptionSnapshot| {
                snapshot
                    .filter(|object| object.object_identifier() == sub.monitored_object_identifier)
                    .or_else(|| db.as_deref()?.get(&sub.monitored_object_identifier))
            };
            // One read per object in this context; all selected values and
            // companions share this DB/snapshot borrow, never a cross-context cache.
            let mut reads = MultipleReads::default();
            for sub in subscriptions {
                if let Some(object) = object_of(sub) {
                    reads.capture_source(object, sub);
                }
            }
            // Untimestamped references qualify now; timestamped ones are decided
            // under the table guard below, against their captured history.
            let mut candidates = Vec::new();
            // Untimestamped references read now, qualifying or not: this
            // fanout takes any owed mark they hold (#1038).
            let mut evaluated = Vec::new();
            for sub in subscriptions {
                let Some(object) = object_of(sub) else {
                    continue;
                };
                if sub.timestamped {
                    // A producer snapshot's changes were captured at the
                    // producer; only a database preparation adds current state.
                    let current = snapshot
                        .is_none()
                        .then(|| reads.read(object, sub))
                        .flatten();
                    candidates.push((sub, Err(current)));
                    continue;
                }
                evaluated.push(sub);
                let Some(prepared) =
                    reads.prepare(object, sub, sub.last_notified_observation.as_ref(), force)
                else {
                    continue;
                };
                let Some(completion) = sub.prepare_completion() else {
                    continue;
                };
                candidates.push((sub, Ok((prepared.values, prepared.observation, completion))));
            }

            // Established lock order: DB read -> table read -> timed store. No
            // object callback runs under the table guard. Each prepared value
            // owns its own check; a live sibling with a failed read cannot
            // authorize a stale value.
            let context = subscriptions[0]
                .key()
                .multiple_context()
                .expect("Multiple snapshot")
                .clone();
            let (retained, untimed, subscriber_max_apdu, owners, store, any_timestamped) = {
                let table = cov_table.read().await;
                // A confirmed context has at most one outstanding report, and
                // the next one has to batch everything held meanwhile (#896).
                // That report's Ack, or the first fanout after a hold-off, sends
                // it; nothing is drained until then.
                if !table.context_idle(&context, subscriptions) {
                    // Pending timestamped changes are retried when a
                    // hold-off ends, not after the backstop's usual wait.
                    if let Some(until) = table.context_hold_until(&context) {
                        table.timed().hold_until(&context, until);
                    }
                    return;
                }
                // A failed report's changes can sit on any object of the
                // context. The first fanout after its hold-off hands the whole
                // context to one follow-up instead of reporting only its own
                // object.
                if table.take_owed_context(&context) {
                    table.revisits().request(
                        table
                            .multiple_context_references(&context)
                            .map(|sub| sub.key().clone()),
                    );
                    return;
                }
                let now = runtime_clock::now();
                let store = table.timed().clone();
                // An unconfirmed context sends one report at a time, so no later
                // report overtakes the parts of this one (#986, #1038); a
                // confirmed context is held by its outstanding report instead. A
                // fanout that finds a report going out leaves its changes
                // queued, and that report hands the context to a follow-up when
                // done. A context without timestamped references keeps the
                // turn only for a report of several parts, below.
                let any_timestamped = table
                    .multiple_context_references(&context)
                    .any(|sub| sub.timestamped);
                if !context.confirmed {
                    let keys = table
                        .multiple_context_references(&context)
                        .map(|sub| sub.key().clone())
                        .collect();
                    match SendTurn::begin(&store, &context, table.revisits(), keys) {
                        Some(taken) => turn = Some(taken),
                        None => return,
                    }
                }
                let claim = claim.insert(TimedClaim::new(store.clone()));
                // Since when each evaluated untimestamped reference is owed. A
                // mark taken here without a report to carry it is settled: the
                // reference had nothing left to report.
                let owed: HashMap<_, _> = {
                    let mut timed = store.lock();
                    evaluated
                        .iter()
                        .filter_map(|sub| {
                            Some((sub.key(), timed.take_owed(sub.key(), sub.generation())?))
                        })
                        .collect()
                };
                let mut retained = Vec::new();
                for (sub, prepared) in candidates {
                    let Some(remaining) = table
                        .remaining_lifetime(sub, now)
                        .and_then(crate::cov::CovTimeRemaining::wire_seconds)
                    else {
                        continue;
                    };
                    let current = match prepared {
                        Ok((values, baseline, completion)) => {
                            claim.add_untimed(
                                sub.key().clone(),
                                sub.generation(),
                                owed.get(sub.key()).copied(),
                            );
                            retained.push((sub.clone(), values, baseline, completion, remaining));
                            continue;
                        }
                        Err(current) => current,
                    };
                    let Some(completion) = sub.prepare_completion() else {
                        continue;
                    };
                    // Captured changes carry their own commit times. The current
                    // state is conveyed as well only when it differs from the last
                    // captured or conveyed observation (a change no producer
                    // captured, such as a raw database mutation), stamped with
                    // this preparation's clock.
                    let mut timed = store.lock();
                    let (incarnation, mut changes) = timed.drain(sub.key(), sub.generation());
                    // The store baseline is the newest captured or conveyed state;
                    // a returned older change can sit at the tail of `changes`.
                    let baseline = timed
                        .baseline(sub.key(), sub.generation())
                        .cloned()
                        .or_else(|| changes.last().map(|change| change.observation().clone()))
                        .or_else(|| sub.last_notified_observation.clone());
                    // An admission capture already supplied the initial report.
                    let force = force
                        && changes.is_empty()
                        && timed.baseline(sub.key(), sub.generation()).is_none();
                    let current = current
                        .filter(|current| force || reads.reports(current, baseline.as_ref()));
                    match (current, clock_frame) {
                        (Some(prepared), Some(frame)) => {
                            let values = reads.with_flags_companion(
                                &sub.monitored_object_identifier,
                                prepared.values,
                            );
                            changes.push(timed.adopt(
                                sub.key(),
                                sub.generation(),
                                TimedChange::new(frame, values, prepared.observation),
                            ));
                        }
                        (Some(_), None) => warn!(
                            "Skipping timestamped COV-multiple change without a valid Device clock"
                        ),
                        (None, _) => {}
                    }
                    drop(timed);
                    let Some(last) = changes.last().map(|change| change.observation().clone())
                    else {
                        continue;
                    };
                    claim.add(sub.key().clone(), incarnation, changes);
                    retained.push((sub.clone(), Vec::new(), last, completion, remaining));
                }
                // Every notification to a context conveys all of its pending
                // timestamped changes (§§13.17.1.1, 13.18.1.1), including those
                // of references on objects that did not change now. A confirmed
                // context starts none while a report is outstanding, so this
                // holds for it too. Captured values need no object read.
                let mut untimed = HashSet::new();
                for other in table.multiple_context_references(&context) {
                    if !other.timestamped {
                        untimed.insert((
                            other.monitored_object_identifier,
                            other.monitored_property,
                            other.monitored_property_array_index,
                        ));
                        continue;
                    }
                    if subscriptions.iter().any(|sub| sub.key() == other.key()) {
                        continue;
                    }
                    let Some(remaining) = table
                        .remaining_lifetime(other, now)
                        .and_then(crate::cov::CovTimeRemaining::wire_seconds)
                    else {
                        continue;
                    };
                    let Some(completion) = other.prepare_completion() else {
                        continue;
                    };
                    let (incarnation, changes) =
                        store.lock().drain(other.key(), other.generation());
                    let Some(last) = changes.last().map(|change| change.observation().clone())
                    else {
                        continue;
                    };
                    claim.add(other.key().clone(), incarnation, changes);
                    retained.push((other.clone(), Vec::new(), last, completion, remaining));
                }
                let subscriber_max_apdu = table
                    .multiple_context_references(&context)
                    .find_map(|sub| sub.subscriber_max_apdu());
                // A live timestamped reference that conveys no change in the
                // last notification still times its field when a sibling
                // carries it (#987). Changes drained now can all go out in an
                // earlier part (#1008), so every live one is an owner; one
                // whose latest change the last notification carries times
                // its field itself.
                let owners: HashMap<Coordinate, (crate::cov::CovSubscriptionKey, u64)> = table
                    .multiple_context_references(&context)
                    .filter(|other| {
                        other.timestamped
                            && table
                                .remaining_lifetime(other, now)
                                .and_then(crate::cov::CovTimeRemaining::wire_seconds)
                                .is_some()
                    })
                    .map(|other| {
                        (
                            (
                                other.monitored_object_identifier,
                                other.monitored_property,
                                other.monitored_property_array_index,
                            ),
                            (other.key().clone(), other.generation()),
                        )
                    })
                    .collect();
                // References an earlier report left owed go first, so a
                // split report cannot keep deferring them (#1038).
                retained.sort_by_key(|(sub, ..)| !owed.contains_key(sub.key()));
                (
                    retained,
                    untimed,
                    subscriber_max_apdu,
                    owners,
                    store,
                    any_timestamped,
                )
            };
            let Some((representative, _, _, _, time_remaining)) = retained.first() else {
                return;
            };
            let representative = representative.clone();
            let envelope = Envelope {
                subscriber_process_identifier: representative.subscriber_process_identifier,
                initiating_device_identifier: device_oid,
                time_remaining: *time_remaining,
            };
            let parts: Vec<_> = retained
                .iter()
                .map(|(sub, values, _, _, _)| (sub, values.as_slice()))
                .collect();
            // The untimestamped values travel with the last notification, so
            // the history bound keeps room for them (#986).
            store.lock().note_reserve(&context, untimed_octets(&parts));
            let limit = request_limit(
                config.max_apdu_length,
                subscriber_max_apdu,
                representative.issue_confirmed_notifications,
            );
            // Times a field of a timestamped reference that conveys no change
            // now, when a sibling carries it (#987).
            let stamp = |coordinate: &Coordinate, value: &[u8]| {
                let Some((key, generation)) = owners.get(coordinate) else {
                    return FieldStamp::Unowned;
                };
                // The clock is read only for a value no capture recorded, and
                // only on the database path, under its read guard.
                let now = || {
                    db.as_deref()?
                        .clock_frame()
                        .filter(|frame| frame.is_valid_actual_datetime())
                };
                match store.lock().field_time(key, *generation, value, now) {
                    FieldTiming::NotLive => FieldStamp::Unowned,
                    FieldTiming::NoTime => FieldStamp::Withhold,
                    FieldTiming::At(seq, frame) => FieldStamp::At((seq, frame)),
                }
            };
            let content = ReportContent {
                envelope: &envelope,
                retained: &parts,
                reads: &reads,
                untimed: &untimed,
                stamp: &stamp,
            };
            let claim = claim.take().expect("claim created under the table guard");
            let parts = split(&content, claim, limit);
            // Untimestamped values a single notification carries need no
            // turn; reports of them may go out side by side, as before.
            if !any_timestamped && parts.len() < 2 {
                turn = None;
            }
            let last_notified: Vec<_> = retained
                .into_iter()
                .map(|(sub, _, baseline, completion, _)| (sub, baseline, completion))
                .collect();
            (parts, last_notified, representative)
        };

        // From the final live decision through fresh admission there is no await.
        if budget.is_exhausted() {
            counters
                .notifications_throttled_fanout
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        if representative.issue_confirmed_notifications {
            let max_apdu_length = apdu::max_apdu_header_at_or_below(config.max_apdu_length)
                .expect("validated local APDU capacity");
            let mut parts = parts.into_iter();
            // Every timestamped value was too large to send, and nothing
            // else changed.
            let Some(first) = parts.next() else {
                return;
            };
            // Only the oldest part goes now. The rest returns to its queue,
            // uncounted, once this report holds the context, and the Ack's
            // follow-up sends the next part (#986). Deferred, not dropped:
            // requeueing them must not let the bound evict what this report
            // planned to send. The values of a change sent one per
            // notification rejoin there as one change (#1090). Untimestamped
            // references of later parts are owed meanwhile, and the follow-up
            // reads their values afresh, so a newer change to one goes in
            // their place (#1038).
            let deferred = parts.map(|part| part.claim.without_eviction()).collect();
            let observations = carried(&first, &last_notified, true);
            let notification = first.notification;
            Self::send_confirmed_cov(
                handles,
                budget,
                ConfirmedReport {
                    service: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                    route: representative,
                    // The newest prepared ticket postdates every carried
                    // reference's baseline, so it completes them all.
                    completion: observations
                        .iter()
                        .map(|(_, _, completion)| *completion)
                        .max_by_key(|completion| completion.ticket())
                        .expect("a carried reference"),
                    observations: observations
                        .into_iter()
                        .map(|(sub, observation, _)| (sub, observation))
                        .collect(),
                    claim: Some(first.claim),
                    deferred,
                },
                |invoke_id| {
                    Self::encode_confirmed_cov_multiple_apdu(
                        &notification,
                        invoke_id,
                        max_apdu_length,
                    )
                },
            )
            .await;
        } else {
            // Every part goes out now, oldest changes first, each retired as
            // it is sent. A failure or an exhausted budget stops the rest,
            // which returns to its queue for a later notification (#986).
            let mut delivered = Vec::new();
            let mut parts = parts.into_iter();
            let (mut stopped, mut sent) = (None, false);
            for part in parts.by_ref() {
                // Communication may have been restricted since the fanout began
                // (Clause 16.1). Stop; re-enabling it rearms the backstop, which
                // sends the parts left queued.
                if handles.ctx.comm_state.initiation_restricted() {
                    stopped = Some(part);
                    break;
                }
                let buf = match Self::encode_unconfirmed_cov_multiple_apdu(&part.notification) {
                    Ok(buf) => buf,
                    Err(e) => {
                        warn!(error = %e, "Failed to encode unconfirmed COVNotificationMultiple");
                        stopped = Some(part);
                        break;
                    }
                };

                if !budget.try_consume(buf.len()) {
                    counters
                        .notifications_throttled_fanout
                        .fetch_add(1, Ordering::Relaxed);
                    stopped = Some(part);
                    break;
                }

                counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
                counters
                    .notifications_unconfirmed
                    .fetch_add(1, Ordering::Relaxed);
                counters
                    .notification_bytes_sent
                    .fetch_add(buf.len() as u64, Ordering::Relaxed);

                if let Err(e) = Self::send_cov_apdu(network, &buf, &representative, false).await {
                    warn!(error = %e, "Failed to send COVNotificationMultiple");
                    stopped = Some(part);
                    break;
                }
                // A reference whose latest change, or untimestamped values,
                // this part carried is complete once it is sent, whatever
                // becomes of the parts after it (#1008, #1038).
                delivered.extend(carried(&part, &last_notified, false));
                part.claim.commit();
                sent = true;
            }
            // A report that began going out owes the untimestamped values of
            // the parts it did not send, for the backstop (#1038).
            if sent {
                for part in stopped.into_iter().chain(parts) {
                    drop(part.claim.owing());
                }
            }
            if !delivered.is_empty() {
                let mut table = cov_table.write().await;
                for (snapshot, pv, completion) in delivered {
                    table.complete_observation(&snapshot, completion, pv);
                }
            }
            drop(turn);
        }
    }
}

/// A reference a report carries, with the observation its delivery
/// establishes and the ticket that completes it.
type Carried = (
    CovSubscriptionSnapshot,
    crate::cov::CovObservation,
    crate::cov::PreparedCovCompletion,
);

/// The references `part` completes, each with its observation: those it
/// finishes, at their latest drained change or prepared state, and, when
/// `every` (a confirmed part's Ack completes all it carries), each other
/// timestamped reference at the last change of it the part carries.
fn carried(part: &Part, last_notified: &[Carried], every: bool) -> Vec<Carried> {
    last_notified
        .iter()
        .filter_map(|(sub, baseline, completion)| {
            if part.finishes.contains(sub.key()) {
                return Some((sub.clone(), baseline.clone(), *completion));
            }
            let (_, change) = part
                .claim
                .last_changes()
                .filter(|_| every)
                .find(|(key, _)| *key == sub.key())?;
            Some((sub.clone(), change.observation().clone(), *completion))
        })
        .collect()
}
