use super::event_message_policy::intrinsic_event_message_text;
use super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::event_timestamp::{
    confirm_event_timestamp, sample_event_timestamp, stage_event_timestamp, SampledEventClock,
};
use super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::event::{EventTransition, EventTransitionCommit, TransitionOutcome};
use bacnet_objects::notification_class::local_day_and_time;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::EventType;
use bacnet_types::primitives::{BACnetTimeStamp, Time};

use crate::event_enrollment::{CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult};

#[path = "event_recipient_lookup.rs"]
mod recipient_lookup;
use recipient_lookup::matched_recipients_or_log;

/// One exact transition-coordinate projection from object-owned event history.
#[derive(Debug, Clone, PartialEq)]
struct CommittedHistorySnapshot {
    timestamp: BACnetTimeStamp,
    message_text: Option<String>,
}

enum CommittedMessageProjection {
    RequiredProperty,
    IntentionallyAbsent,
}

/// The source boundary for the notification's timestamp and message.
enum NotificationHistorySource {
    /// Legacy/default objects sample their timestamp when distribution begins.
    SendTime,
    /// Atomic transitions use the exact history coordinate read after commit.
    Committed {
        snapshot: CommittedHistorySnapshot,
        recipient_clock: SampledEventClock,
    },
}

pub(super) struct NotificationTransition {
    change: EventStateChange,
    event_type: EventType,
    history_source: NotificationHistorySource,
    ack_required: Option<bool>,
}

impl From<(EventStateChange, EventType)> for NotificationTransition {
    fn from((change, event_type): (EventStateChange, EventType)) -> Self {
        Self {
            change,
            event_type,
            history_source: NotificationHistorySource::SendTime,
            ack_required: None,
        }
    }
}

/// One built-in intrinsic transition committed under the database write guard.
pub(super) struct CommittedIntrinsicTransition {
    pub(super) change: EventStateChange,
    pub(super) event_type: EventType,
    pub(super) distribute: bool,
    history_snapshot: CommittedHistorySnapshot,
    recipient_clock: SampledEventClock,
    ack_required: bool,
}

impl From<CommittedIntrinsicTransition> for NotificationTransition {
    fn from(committed: CommittedIntrinsicTransition) -> Self {
        Self {
            change: committed.change,
            event_type: committed.event_type,
            history_source: NotificationHistorySource::Committed {
                snapshot: committed.history_snapshot,
                recipient_clock: committed.recipient_clock,
            },
            ack_required: Some(committed.ack_required),
        }
    }
}

/// A server-ready intrinsic outcome under its declared object contract.
///
/// Built-ins carry their atomic commit snapshot. Legacy implementations have
/// already mutated state during evaluation and retain send-time policy and
/// timestamp sampling.
pub(super) enum ResolvedIntrinsicTransition {
    Committed(CommittedIntrinsicTransition),
    Legacy(TransitionOutcome),
}

impl ResolvedIntrinsicTransition {
    pub(super) fn distribute(&self) -> bool {
        match self {
            Self::Committed(committed) => committed.distribute,
            Self::Legacy(outcome) => outcome.distribute,
        }
    }
}

impl From<ResolvedIntrinsicTransition> for NotificationTransition {
    fn from(transition: ResolvedIntrinsicTransition) -> Self {
        match transition {
            ResolvedIntrinsicTransition::Committed(committed) => committed.into(),
            ResolvedIntrinsicTransition::Legacy(outcome) => {
                (outcome.change, outcome.event_type).into()
            }
        }
    }
}

/// Project an alarm/event priority onto the NPDU Network Priority.
///
/// Clause 13.2.5.4: "the Network Priority as defined in Clause 6.2.2 shall be
/// set as a function of the alarm and event priority as defined in Table
/// 13-6". Lower event priority is more urgent: 00–63 is a Life Safety
/// message, 64–127 Critical Equipment, 128–191 Urgent, 192–255 Normal.
pub(super) fn network_priority_for_event(priority: u8) -> NetworkPriority {
    match priority {
        0..=63 => NetworkPriority::LIFE_SAFETY,
        64..=127 => NetworkPriority::CRITICAL_EQUIPMENT,
        128..=191 => NetworkPriority::URGENT,
        192..=255 => NetworkPriority::NORMAL,
    }
}

/// Operational fallback for recipient-window filtering in clockless mode.
///
/// This uses system UTC only to avoid dropping an alarm while no Device clock
/// is advertised; it does not create a Device DateTime or change wire
/// timestamp selection.
fn system_utc_recipient_filter_time(now: Duration) -> (u8, Time) {
    let (today_bit, mut current_time) = local_day_and_time(now.as_secs(), 0);
    current_time.hundredths = (now.subsec_millis() / 10) as u8;
    (today_bit, current_time)
}

/// Read one exact committed transition coordinate through the object contract.
///
/// Required properties are projected while the caller still owns the database
/// write guard. A malformed or incomplete projection is not equivalent to a
/// missing message/timestamp: the committed transition remains local, but no
/// outward frame can be built from an unproven history snapshot. Event
/// Enrollment is the explicit exception for message lookup because that object
/// intentionally has no `Event_Message_Texts` property.
fn read_committed_history_snapshot(
    object: &dyn BACnetObject,
    coordinate: EventTransition,
    message_projection: CommittedMessageProjection,
) -> Option<CommittedHistorySnapshot> {
    let array_index = u32::try_from(coordinate.index() + 1)
        .expect("three event transition coordinates fit in u32");
    let PropertyValue::ApplicationData(encoded_timestamp) = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(array_index))
        .ok()?
    else {
        return None;
    };
    let (timestamp, consumed) = decode_timestamp_choice(&encoded_timestamp, 0).ok()?;
    if consumed != encoded_timestamp.len() {
        return None;
    }

    let message_text = match message_projection {
        CommittedMessageProjection::RequiredProperty => {
            let PropertyValue::CharacterString(message_text) = object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(array_index))
                .ok()?
            else {
                return None;
            };
            (!message_text.is_empty()).then_some(message_text)
        }
        CommittedMessageProjection::IntentionallyAbsent => None,
    };

    Some(CommittedHistorySnapshot {
        timestamp,
        message_text,
    })
}

/// Project an already-committed Event Enrollment result without another
/// transition commit or timestamp sample.
pub(super) fn resolve_committed_event_enrollment_transition(
    db: &ObjectDatabase,
    committed: CommittedEventEnrollmentDelivery,
) -> Option<(ObjectIdentifier, bool, NotificationTransition)> {
    let (oid, change, event_type, distribute) = match committed.result {
        CommittedEventEnrollmentResult::Normal(result) => (
            result.enrollment_oid,
            result.change,
            result.event_type,
            result.distribute,
        ),
        CommittedEventEnrollmentResult::Reliability(result) => {
            let object = db.get(&result.enrollment_oid)?;
            let PropertyValue::Enumerated(configured_event_type) = object
                .read_property(PropertyIdentifier::EVENT_TYPE, None)
                .ok()?
            else {
                return None;
            };
            let configured_event_type = EventType::from_raw(configured_event_type);
            if ![
                EventType::OUT_OF_RANGE,
                EventType::FLOATING_LIMIT,
                EventType::CHANGE_OF_STATE,
                EventType::CHANGE_OF_BITSTRING,
                EventType::CHANGE_OF_VALUE,
                EventType::NONE,
            ]
            .contains(&configured_event_type)
            {
                return None;
            }
            let event_type = result.event_type(configured_event_type)?;
            let change = result.state_change.clone()?;
            (result.enrollment_oid, change, event_type, result.distribute)
        }
    };

    let coordinate = change.transition();
    let history_snapshot = db.get(&oid).and_then(|object| {
        read_committed_history_snapshot(
            object,
            coordinate,
            CommittedMessageProjection::IntentionallyAbsent,
        )
    })?;
    Some((
        oid,
        distribute,
        NotificationTransition {
            change,
            event_type,
            history_source: NotificationHistorySource::Committed {
                snapshot: history_snapshot,
                recipient_clock: committed.recipient_clock,
            },
            ack_required: Some(committed.ack_required),
        },
    ))
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Resolve policy and atomically commit one intrinsic proposal.
    pub(super) fn commit_intrinsic_transition(
        db: &mut ObjectDatabase,
        oid: &ObjectIdentifier,
        outcome: TransitionOutcome,
    ) -> Option<CommittedIntrinsicTransition> {
        let coordinate = outcome.change.transition();
        let notification_class = db
            .get(oid)?
            .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .ok()
            .and_then(|value| match value {
                PropertyValue::Unsigned(number) => Some(number as u32),
                _ => None,
            })
            .unwrap_or(0);
        let (_, ack_required) = resolve_transition_priority_ack(db, notification_class, coordinate);
        let staged_timestamp = stage_event_timestamp(db);
        let message_text = intrinsic_event_message_text(oid, &outcome.change);
        let commit = EventTransitionCommit {
            change: outcome.change.clone(),
            coordinate,
            ack_required,
            timestamp: staged_timestamp.sample.timestamp.clone(),
            message_text: Some(message_text),
        };

        if let Err(error) = db.get_mut(oid)?.commit_event_transition_internal(commit) {
            debug!(%oid, ?error, "Intrinsic transition commit rejected");
            return None;
        }

        let recipient_clock = confirm_event_timestamp(db, staged_timestamp).clock;
        let history_snapshot = match db.get(oid).and_then(|object| {
            read_committed_history_snapshot(
                object,
                coordinate,
                CommittedMessageProjection::RequiredProperty,
            )
        }) {
            Some(snapshot) => snapshot,
            None => {
                debug!(
                    %oid,
                    ?coordinate,
                    "Committed intrinsic history projection rejected; suppressing distribution"
                );
                return None;
            }
        };
        Some(CommittedIntrinsicTransition {
            change: outcome.change,
            event_type: outcome.event_type,
            distribute: outcome.distribute,
            history_snapshot,
            recipient_clock,
            ack_required,
        })
    }

    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to the recipients the object's NotificationClass names.
    /// DCC gates network-message initiation (Clause 16.1), not the local
    /// transition actions in Clause 13.2.2.1.4. The outbound sender below
    /// suppresses distribution while communications are disabled.
    ///
    /// This is the per-write entry point: it probes the detector, which fires
    /// immediately only when `Time_Delay == 0`. For a nonzero delay the probe
    /// seeds a pending transition (returning `None`, so no notification is
    /// sent here) and the one-second [`intrinsic_reporting_task`](Self::start)
    /// advances the countdown and sends the notification on expiry.
    #[cfg(test)]
    pub(super) async fn fire_event_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        oid: &ObjectIdentifier,
        retry_timeout_ms: u64,
    ) {
        Self::fire_event_notifications_with_bindings(
            db,
            network,
            comm_state,
            server_tsm,
            notification_transactions,
            &Arc::new(RwLock::new(DeviceBindingTable::new())),
            oid,
            retry_timeout_ms,
        )
        .await;
    }

    pub(super) async fn fire_event_notifications_with_bindings(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        oid: &ObjectIdentifier,
        retry_timeout_ms: u64,
    ) {
        let resolved = {
            let mut db = db.write().await;
            let (requires_atomic_commit, outcome) = match db.get_mut(oid) {
                Some(object) => (
                    object.intrinsic_reporting_requires_atomic_commit(),
                    object.evaluate_intrinsic_reporting(),
                ),
                None => return,
            };
            outcome.and_then(|outcome| {
                if requires_atomic_commit {
                    Self::commit_intrinsic_transition(&mut db, oid, outcome)
                        .map(ResolvedIntrinsicTransition::Committed)
                } else {
                    Some(ResolvedIntrinsicTransition::Legacy(outcome))
                }
            })
        };

        // A successful built-in commit has already applied the local transition
        // actions; a legacy object applied them during evaluation. Whatever
        // Event_Enable says, only external distribution is gated here:
        // Clause 12.12 defines Event_Enable as enabling and disabling the
        // distribution of notifications, and Clause 13.2.5 places that gate
        // inside the notification-distribution process — downstream of the
        // transition actions, none of which it governs.
        //
        // The shared commit kernel has also stored the selected timestamp and
        // updated Acked_Transitions from the Notification Class policy and
        // stored the selected local message in the transition coordinate.
        if let Some(resolved) = resolved {
            if resolved.distribute() {
                Self::build_and_send_event_notification_with_bindings(
                    db,
                    network,
                    comm_state,
                    server_tsm,
                    notification_transactions,
                    device_bindings,
                    oid,
                    resolved,
                    retry_timeout_ms,
                )
                .await;
            }
        }
    }

    /// Build an `EventNotificationRequest` for a pre-computed transition and
    /// send it to the recipients the object's NotificationClass names.
    ///
    /// Shared by the per-write path and the
    /// periodic `Time_Delay` confirmation path, so both emit identical
    /// notifications. Skipped when DCC is active (comm_state >= 1). Re-reads
    /// `Notification_Class` / `Notify_Type` under a brief `db.write()` guard,
    /// then drops the lock before any network send.
    #[cfg(test)]
    pub(super) async fn build_and_send_event_notification(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
        retry_timeout_ms: u64,
    ) {
        Self::build_and_send_event_notification_with_bindings(
            db,
            network,
            comm_state,
            server_tsm,
            notification_transactions,
            &Arc::new(RwLock::new(DeviceBindingTable::new())),
            oid,
            transition,
            retry_timeout_ms,
        )
        .await;
    }

    pub(super) async fn build_and_send_event_notification_with_bindings(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
        retry_timeout_ms: u64,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }

        let NotificationTransition {
            change,
            event_type,
            history_source,
            ack_required: ack_required_snapshot,
        } = transition.into();
        let system_utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let (notification, recipients) = {
            let mut db = db.write().await;

            let (timestamp, message_text, recipient_clock) = match history_source {
                NotificationHistorySource::SendTime => {
                    let sample = sample_event_timestamp(&mut db);
                    (sample.timestamp, None, sample.clock)
                }
                NotificationHistorySource::Committed {
                    snapshot,
                    recipient_clock,
                } => (snapshot.timestamp, snapshot.message_text, recipient_clock),
            };

            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            let (today_bit, current_time) = match recipient_clock {
                SampledEventClock::Valid(clock_frame) => (
                    clock_frame
                        .day_of_week_bit()
                        .expect("validated ClockFrame has a day of week"),
                    clock_frame.local_time,
                ),
                SampledEventClock::Unavailable => {
                    debug!("Using system UTC to filter recipients without a Device clock");
                    system_utc_recipient_filter_time(system_utc)
                }
                SampledEventClock::Invalid => {
                    debug!(
                        "Using system UTC to filter recipients with an invalid Device clock frame"
                    );
                    system_utc_recipient_filter_time(system_utc)
                }
            };

            let object = match db.get_mut(oid) {
                Some(o) => o,
                None => return,
            };

            let notification_class = object
                .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Unsigned(n) => Some(n as u32),
                    _ => None,
                })
                .unwrap_or(0);

            let notify_type = object
                .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Enumerated(n) => Some(n),
                    _ => None,
                })
                .unwrap_or(NotifyType::ALARM.to_raw());

            let transition = change.transition();

            // Resolve the per-transition Priority and Ack_Required from the
            // referenced NotificationClass (ASHRAE 135-2020 §13.2.1), falling
            // back to the BACnet defaults (Priority 255, no ack) when the
            // class is missing.
            let (priority, resolved_ack_required) =
                resolve_transition_priority_ack(&db, notification_class, transition);
            let ack_required = ack_required_snapshot.unwrap_or(resolved_ack_required);

            let Some(recipients) = matched_recipients_or_log(
                lookup_notification_recipients(
                    &db,
                    notification_class,
                    transition,
                    today_bit,
                    &current_time,
                ),
                notification_class,
                transition,
            ) else {
                return;
            };

            let base_notification = EventNotificationRequest {
                process_identifier: 0,
                initiating_device_identifier: device_oid,
                event_object_identifier: *oid,
                timestamp,
                notification_class,
                priority,
                event_type: event_type.to_raw(),
                message_text,
                notify_type,
                // ack_required is only meaningful for ALARM/EVENT notify types
                // (ACK_NOTIFICATION omits the field on the wire). Per §13.2.1 the
                // value is the NotificationClass's per-transition Ack_Required,
                // not a function of Notify_Type alone.
                ack_required: if notify_type == NotifyType::ACK_NOTIFICATION.to_raw() {
                    false
                } else {
                    ack_required
                },
                from_state: change.from.to_raw(),
                to_state: change.to.to_raw(),
                event_values: None,
            };

            (base_notification, recipients)
        };

        let notification_class = notification.notification_class;
        // Clause 13.2.5.4 sets the NPDU priority from the notification's
        // event priority, on every send of this notification — retries too.
        let network_priority = network_priority_for_event(notification.priority);

        for (recipient, process_id, confirmed) in &recipients {
            let route = match recipient {
                BACnetRecipient::Address(address) => {
                    RecipientRoute::resolve_address(address, |mac| {
                        network.transport().is_broadcast_mac(mac)
                    })
                }
                BACnetRecipient::Device(identifier) => {
                    let resolution = {
                        let table = device_bindings.read().await;
                        table.resolve_at(identifier, Instant::now(), |mac| {
                            network.transport().is_broadcast_mac(mac)
                        })
                    };
                    RecipientRoute::from_device_resolution(resolution)
                }
            };

            if !route.is_deliverable(notification_class) {
                continue;
            }

            // Downgrading to unconfirmed would drop the acknowledgment the
            // recipient was configured to require, so both cases are skips.
            if *confirmed && !route.permits_confirmed() {
                // Clause 6.3 restricts broadcast to Unconfirmed-Request-PDUs.
                warn!(
                    notification_class,
                    "Recipient requests confirmed notifications at a broadcast address; \
                     Clause 6.3 permits only unconfirmed PDUs there, skipping"
                );
                continue;
            }

            let mut targeted = notification.clone();
            targeted.process_identifier = *process_id;

            let mut service_buf = BytesMut::new();
            if let Err(e) = targeted.encode(&mut service_buf) {
                warn!(error = %e, "Failed to encode EventNotification");
                continue;
            }

            let service_bytes = service_buf.freeze();

            if *confirmed {
                // Convert only the unicast route shapes admitted above and
                // fail closed if the route classification changes.
                let Some(ConfirmedRecipientRoute {
                    canonical_peer,
                    local_target,
                    remote,
                    freshness,
                }) = route.into_confirmed()
                else {
                    warn!(
                        notification_class,
                        "Confirmed notification route is unusable"
                    );
                    continue;
                };
                let (operation, result_rx) = match notification_transactions.reserve(
                    canonical_peer,
                    ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                ) {
                    Ok(reservation) => reservation,
                    Err(error) => {
                        warn!(%error, "No free invoke ID for confirmed EventNotification");
                        continue;
                    }
                };
                let id = operation.invoke_id();

                let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length: 1476,
                    invoke_id: id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                    service_request: service_bytes,
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                let network = Arc::clone(network);
                let tsm = Arc::clone(server_tsm);
                let timeout = Duration::from_millis(retry_timeout_ms);
                let apdu_retries = DEFAULT_APDU_RETRIES;
                tokio::spawn(async move {
                    let result = run_notification_worker(
                        operation,
                        result_rx,
                        timeout,
                        apdu_retries,
                        |attempt| {
                            let network = Arc::clone(&network);
                            let tsm = Arc::clone(&tsm);
                            let buf = buf.clone();
                            let local_target = local_target.clone();
                            let remote = remote.clone();
                            async move {
                                if freshness.is_some_and(|freshness| {
                                    !freshness.permits_attempt_at(tokio::time::Instant::now())
                                }) {
                                    debug!(
                                        invoke_id = id,
                                        attempt,
                                        "Observed Device binding expired before notification attempt"
                                    );
                                    return Err(());
                                }
                                let send_result = match (local_target, remote) {
                                    (Some(target), None) => {
                                        network
                                            .send_apdu(&buf, &target, true, network_priority)
                                            .await
                                    }
                                    (None, Some((dnet, dadr, configured_router))) => {
                                        // A Device binding keeps its fixed next hop for
                                        // each permitted attempt. Address recipients retain
                                        // the learned-router/broadcast behavior.
                                        let router = match configured_router {
                                            Some(router) => Some(router),
                                            None if attempt == 0 => {
                                                tsm.lock().await.cached_router(dnet)
                                            }
                                            None => None,
                                        };
                                        match router {
                                            Some(router_mac) => {
                                                network
                                                    .send_apdu_routed(
                                                        &buf,
                                                        dnet,
                                                        &dadr,
                                                        &router_mac,
                                                        true,
                                                        network_priority,
                                                    )
                                                    .await
                                            }
                                            None => {
                                                network
                                                    .send_apdu_routed_via_local_broadcast(
                                                        &buf,
                                                        dnet,
                                                        &dadr,
                                                        true,
                                                        network_priority,
                                                    )
                                                    .await
                                            }
                                        }
                                    }
                                    _ => unreachable!("confirmed route validated before spawn"),
                                };
                                match &send_result {
                                    Ok(()) => debug!(
                                        invoke_id = id,
                                        attempt, "Confirmed EventNotification sent"
                                    ),
                                    Err(error) => warn!(
                                        %error,
                                        attempt, "Confirmed EventNotification send failed"
                                    ),
                                }
                                send_result.map_err(|_| ())
                            }
                        },
                    )
                    .await;
                    match result {
                        NotificationWorkerResult::Ack => {
                            debug!(invoke_id = id, "EventNotification acknowledged");
                        }
                        NotificationWorkerResult::Error => {
                            warn!(invoke_id = id, "EventNotification rejected by recipient");
                        }
                        NotificationWorkerResult::Exhausted => warn!(
                            invoke_id = id,
                            "EventNotification failed after {} retries", apdu_retries
                        ),
                        NotificationWorkerResult::Closed => {}
                    }
                });
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                    service_request: service_bytes,
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                let send_result = match &route {
                    RecipientRoute::LocalUnicast(mac) => {
                        network.send_apdu(&buf, mac, false, network_priority).await
                    }
                    RecipientRoute::BoundLocalUnicast { mac, .. } => {
                        network.send_apdu(&buf, mac, false, network_priority).await
                    }
                    RecipientRoute::LocalBroadcast => {
                        network.broadcast_apdu(&buf, false, network_priority).await
                    }
                    // Carries DNET with DLEN zero, so routers forward it
                    // onto the remote network as a broadcast there.
                    RecipientRoute::RemoteBroadcast(net) => {
                        network
                            .broadcast_to_network(&buf, *net, false, network_priority)
                            .await
                    }
                    // Carries DNET 0xFFFF, which routers forward to every
                    // reachable network. `broadcast_to_network` rejects that
                    // DNET, so it needs its own send.
                    RecipientRoute::GlobalBroadcast => {
                        network
                            .broadcast_global_apdu(&buf, false, network_priority)
                            .await
                    }
                    // DNET/DADR name the recipient; the link DA is the local
                    // broadcast because this non-routing device keeps no
                    // router table (Clause 6.5.3's unknown-router form).
                    RecipientRoute::RemoteUnicast { network: net, mac } => {
                        network
                            .send_apdu_routed_via_local_broadcast(
                                &buf,
                                *net,
                                mac,
                                false,
                                network_priority,
                            )
                            .await
                    }
                    RecipientRoute::BoundRoutedUnicast {
                        network: net,
                        mac,
                        router,
                        ..
                    } => {
                        network
                            .send_apdu_routed(&buf, *net, mac, router, false, network_priority)
                            .await
                    }
                    // Filtered out by `RecipientRoute::is_deliverable` above.
                    RecipientRoute::ContradictoryGlobal
                    | RecipientRoute::UnknownDevice
                    | RecipientRoute::StaleDevice
                    | RecipientRoute::InvalidDevice => continue,
                };

                if let Err(e) = send_result {
                    warn!(
                        error = %e,
                        "Failed to send unconfirmed EventNotification"
                    );
                }
            }
        }
    }
}
