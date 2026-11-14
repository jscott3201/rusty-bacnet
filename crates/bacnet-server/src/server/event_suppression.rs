//! Lifetime counters for event notifications the server did not deliver
//! (#1142), and for the waits that keep their order (#2537).
use super::*;
use bacnet_objects::notification_class::RecipientLookupOutcome;
use std::sync::atomic::AtomicU64;

/// Lifetime totals of event notifications that were not delivered, and of
/// two waits that keep their order (`confirmed_waited_for_invoke_id` and
/// `cov_wait_expired`), each saturating at `u64::MAX`. A new server starts at
/// zero, and the totals stay readable after `stop()`. Fields are sampled
/// independently, not as one atomic aggregate. Each refusal also logs a
/// warning (the forwarding cap's at most once a minute), and each wait for
/// an invoke ID a debug line; these totals are the running signal, not an
/// audit log.
///
/// The first four count transitions (event or acknowledgment notifications)
/// whose Notification Class lookup failed closed, so no recipient received
/// them. A class whose list is empty, or whose destinations all filter the
/// transition out by day, time or transition, is configured behaviour and is
/// not counted. Neither are notifications held back by DeviceCommunicationControl
/// or Event_Enable; DCC holds a confirmed one back at a retry too, ending it
/// there uncounted.
///
/// The next three count destinations that matched the transition but were
/// skipped while their route was resolved, once per destination: the rest of
/// the transition's destinations are still served. They are grouped by what
/// fixes them: a Device recipient the server holds no current address for,
/// a recipient that can never be routed as written, and a confirmed
/// recipient at a broadcast or other group address. The warning logged with
/// each skip names the finer reason.
///
/// The next three count confirmed notifications to one recipient: the first,
/// those that waited for an invoke ID, which are not dropped (each logged at
/// debug), and the two after it, those never acknowledged.
///
/// The next one counts unconfirmed notifications whose send failed at the
/// transport, once per destination. The transition's other destinations are
/// still served. There is no counter for a notification that fails to
/// encode: the committed payload and message text are validated before the
/// destinations are walked, so a well-formed transition always encodes, and
/// the send loop only logs and skips if that invariant is ever broken.
///
/// The next three concern the Notification Forwarder objects (#1225): copies
/// too large to send unsegmented, received notifications no forwarder took,
/// and destinations past the cap on one notification's copies (#1259).
///
/// The next one counts received notifications the Event Logs that collect
/// them would have taken but for their source's allowance (#1346).
///
/// The last four concern the lanes each destination's notifications wait in
/// to go out in order (#2537): notifications dropped at a full lane, with
/// too many waiting in all, or with too many destinations waiting, and
/// changes whose COV notifications stopped waiting for their event
/// notifications at the APDU timeout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventNotificationCounters {
    /// No Notification Class object has the class number the event object
    /// names.
    pub notification_class_missing: u64,
    /// The class exists but reading its Recipient_List failed.
    pub recipient_list_unavailable: u64,
    /// The class's Recipient_List did not decode as a whole; no decodable
    /// prefix is routed.
    pub recipient_list_invalid: u64,
    /// The class serves more than
    /// [`MAX_RECIPIENT_LIST_DESTINATIONS`](bacnet_objects::notification_class::MAX_RECIPIENT_LIST_DESTINATIONS)
    /// destinations, which only a custom Notification Class object can do.
    pub recipient_list_too_long: u64,
    /// Device recipients skipped because the server has no current binding
    /// for them: none was configured or observed, or the observed one has
    /// expired, and looking for the device found none. Such a recipient's
    /// notification waits at the head of its lane for the I-Am a targeted
    /// Who-Is asks for (#1368, #2537), the device's later notifications
    /// behind it; when the device stays silent past the APDU timeout (a
    /// minute at most), each of them counts here once. One counts at once
    /// when no Who-Is may go out for it (within a minute of one that drew
    /// nothing, or with 256 devices already being looked for). Both clear
    /// once the device's I-Am is observed again or a binding is configured.
    /// A confirmed notification whose observed binding expires before an
    /// attempt (a retry, or a first one that waited for its invoke ID) ends
    /// there, its invoke ID freed, and counts here rather than in
    /// `confirmed_unanswered` (#1371).
    pub device_recipient_unbound: u64,
    /// Recipients that cannot be routed as configured, so no binding or
    /// retry delivers them: a Device recipient whose identifier is not a
    /// Device object, or whose binding is unusable on this link, and an
    /// address that puts a MAC on the global broadcast network (65535),
    /// which names neither a broadcast nor one device.
    pub recipient_unroutable: u64,
    /// Recipients configured for confirmed notifications at a broadcast
    /// address (local, remote or global, the link's own broadcast MAC
    /// included), or at any other group address the link reports
    /// ([`TransportPort::is_group_destination`], such as a B/IP multicast
    /// address, #1493). Clause 6.3 lets only unconfirmed requests be
    /// broadcast, and sending one unconfirmed would drop the acknowledgment
    /// the destination asks for. No invoke ID is reserved for them.
    pub confirmed_broadcast_recipient: u64,
    /// Confirmed notifications that found every invoke ID in use and waited
    /// for one to come back, counted once each as its wait starts (#2537).
    /// None is dropped for want of an ID: it waits in its destination's
    /// lane and holds up only that destination.
    pub confirmed_waited_for_invoke_id: u64,
    /// Confirmed notifications the recipient answered with an Error, Reject
    /// or Abort.
    pub confirmed_rejected: u64,
    /// Confirmed notifications that drew no acknowledgment after the last
    /// retry, including attempts whose send failed locally.
    pub confirmed_unanswered: u64,
    /// Unconfirmed notifications the transport refused to send, one per
    /// destination: a unicast, a broadcast or a routed send that returned an
    /// error. Nothing is retried. Confirmed sends that fail locally count in
    /// `confirmed_unanswered` instead.
    pub unconfirmed_send_failed: u64,
    /// Notifications not sent to one destination because their APDU is
    /// longer than the local APDU capacity; notifications are never sent
    /// segmented. A forwarded copy of a notification that arrived segmented
    /// is the usual case. The other destinations are still served.
    pub apdu_too_large: u64,
    /// Received ConfirmedEventNotification and UnconfirmedEventNotification
    /// requests that decoded but that no Notification Forwarder object took.
    /// A confirmed one is still acknowledged. One sent by global broadcast,
    /// which forwarders ignore, and a retransmitted confirmed one, which is
    /// acknowledged without being offered to them again, are not counted.
    pub received_not_forwarded: u64,
    /// Destinations a notification was not forwarded to because it already
    /// had [`MAX_FORWARDED_DESTINATIONS`] copies on their way across this
    /// device's forwarders, one per destination dropped. Only copies that
    /// would be sent take room, so a destination the loop rules refuse is
    /// neither counted here nor against the cap. Destinations go in
    /// forwarder order, then list order, so the ones dropped are the last.
    /// A capped notification logs a warning at most once a minute,
    /// process-wide, and at debug level otherwise.
    pub forwarding_cap_dropped: u64,
    /// Received ConfirmedEventNotification and UnconfirmedEventNotification
    /// requests not logged because their source had used its allowance of
    /// [`RECEIVED_EVENT_LOG_RATE`] records
    /// in the current second, one per notification whatever the number of
    /// logs. Notifications no Event Log collects are not counted.
    pub received_not_logged: u64,
    /// Notifications dropped because their destination's lane already held
    /// 64 waiting behind the one it is sending (#2537): a destination that
    /// stalls, a Device recipient waiting for its I-Am included, drops its
    /// own newest notifications, and the next From State it is sent shows
    /// the gap.
    pub lane_full: u64,
    /// Notifications dropped because 1,024 were already waiting across
    /// every destination's lane.
    pub lanes_entries_full: u64,
    /// Notifications dropped because 256 destinations already had a lane:
    /// one more destination waits for one of them to empty.
    pub lanes_full: u64,
    /// Changes whose COV notifications went out before every event
    /// notification the change made was handed to the network, because the
    /// wait for them ran past the APDU timeout (#889, #2537), or, for an
    /// intrinsic-reporting sweep pass that found 64 others waiting and went
    /// at once, because its own notifications were still waiting.
    pub cov_wait_expired: u64,
}

/// One undelivered event notification, or one that waited for an invoke ID,
/// as counted in [`EventNotificationCounters`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventSuppression {
    NotificationClassMissing,
    RecipientListUnavailable,
    RecipientListInvalid,
    RecipientListTooLong,
    DeviceRecipientUnbound,
    RecipientUnroutable,
    ConfirmedBroadcastRecipient,
    ConfirmedWaitedForInvokeId,
    ConfirmedRejected,
    ConfirmedUnanswered,
    UnconfirmedSendFailed,
    ApduTooLarge,
    ReceivedNotForwarded,
    ForwardingCapDropped,
    ReceivedNotLogged,
}

impl EventSuppression {
    /// The counter a lookup outcome moves, or `None` when the outcome
    /// delivers or is configured behaviour.
    pub(super) fn from_lookup(outcome: &RecipientLookupOutcome) -> Option<Self> {
        match outcome {
            RecipientLookupOutcome::NotificationClassMissing => {
                Some(Self::NotificationClassMissing)
            }
            RecipientLookupOutcome::RecipientListUnavailable => {
                Some(Self::RecipientListUnavailable)
            }
            RecipientLookupOutcome::RecipientListInvalid => Some(Self::RecipientListInvalid),
            RecipientLookupOutcome::RecipientListTooLong => Some(Self::RecipientListTooLong),
            RecipientLookupOutcome::NoConfiguredDestinations
            | RecipientLookupOutcome::NoMatchingDestinations
            | RecipientLookupOutcome::Matched(_) => None,
        }
    }
}

/// The server's shared storage behind [`EventNotificationCounters`].
#[derive(Debug, Default)]
pub(crate) struct EventSuppressions {
    counts: [AtomicU64; 15],
    /// The throttle on the warning for a notification that reached the
    /// forwarding cap, one per server (#1950).
    pub(crate) cap_warning: bacnet_transport::warn_throttle::WarnThrottle,
}

impl EventSuppressions {
    pub(crate) fn record(&self, suppression: EventSuppression) {
        // Relaxed and independent, like the other server counters: no
        // ordering with the send path is promised.
        let _ = self.counts[suppression as usize].try_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |n| Some(n.saturating_add(1)),
        );
    }

    pub(crate) fn snapshot(&self) -> EventNotificationCounters {
        let [
            notification_class_missing,
            recipient_list_unavailable,
            recipient_list_invalid,
            recipient_list_too_long,
            device_recipient_unbound,
            recipient_unroutable,
            confirmed_broadcast_recipient,
            confirmed_waited_for_invoke_id,
            confirmed_rejected,
            confirmed_unanswered,
            unconfirmed_send_failed,
            apdu_too_large,
            received_not_forwarded,
            forwarding_cap_dropped,
            received_not_logged,
        ] = self.counts.each_ref().map(|n| n.load(Ordering::Relaxed));
        EventNotificationCounters {
            notification_class_missing,
            recipient_list_unavailable,
            recipient_list_invalid,
            recipient_list_too_long,
            device_recipient_unbound,
            recipient_unroutable,
            confirmed_broadcast_recipient,
            confirmed_waited_for_invoke_id,
            confirmed_rejected,
            confirmed_unanswered,
            unconfirmed_send_failed,
            apdu_too_large,
            received_not_forwarded,
            forwarding_cap_dropped,
            received_not_logged,
            // The lanes keep their own (#2537); see
            // `BACnetServer::event_notification_counters`.
            lane_full: 0,
            lanes_entries_full: 0,
            lanes_full: 0,
            cov_wait_expired: 0,
        }
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Sample the lifetime totals of undelivered event notifications,
    /// including after `stop()`. See [`EventNotificationCounters`] for what
    /// each field counts and what is left out.
    pub fn event_notification_counters(&self) -> EventNotificationCounters {
        let mut counters = self.event_suppressions.snapshot();
        // The lanes count their own drops and expired waits, with the lanes,
        // which a server that sends no event notification never makes.
        if let Some(lanes) = super::event_send::made_lanes(&self.notification_transactions) {
            lanes.counts().fill(&mut counters);
        }
        counters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [EventSuppression; 15] = [
        EventSuppression::NotificationClassMissing,
        EventSuppression::RecipientListUnavailable,
        EventSuppression::RecipientListInvalid,
        EventSuppression::RecipientListTooLong,
        EventSuppression::DeviceRecipientUnbound,
        EventSuppression::RecipientUnroutable,
        EventSuppression::ConfirmedBroadcastRecipient,
        EventSuppression::ConfirmedWaitedForInvokeId,
        EventSuppression::ConfirmedRejected,
        EventSuppression::ConfirmedUnanswered,
        EventSuppression::UnconfirmedSendFailed,
        EventSuppression::ApduTooLarge,
        EventSuppression::ReceivedNotForwarded,
        EventSuppression::ForwardingCapDropped,
        EventSuppression::ReceivedNotLogged,
    ];

    #[test]
    fn each_suppression_moves_only_its_own_field() {
        let counters = EventSuppressions::default();
        for (step, suppression) in ALL.into_iter().enumerate() {
            for _ in 0..=step {
                counters.record(suppression);
            }
        }
        assert_eq!(
            counters.snapshot(),
            EventNotificationCounters {
                notification_class_missing: 1,
                recipient_list_unavailable: 2,
                recipient_list_invalid: 3,
                recipient_list_too_long: 4,
                device_recipient_unbound: 5,
                recipient_unroutable: 6,
                confirmed_broadcast_recipient: 7,
                confirmed_waited_for_invoke_id: 8,
                confirmed_rejected: 9,
                confirmed_unanswered: 10,
                unconfirmed_send_failed: 11,
                apdu_too_large: 12,
                received_not_forwarded: 13,
                forwarding_cap_dropped: 14,
                received_not_logged: 15,
                ..EventNotificationCounters::default()
            }
        );
    }

    #[test]
    fn counters_saturate_at_u64_max() {
        let counters = EventSuppressions::default();
        for counter in &counters.counts {
            counter.store(u64::MAX - 1, Ordering::Relaxed);
        }
        for _ in 0..3 {
            for suppression in ALL {
                counters.record(suppression);
            }
        }
        assert!(
            counters
                .counts
                .iter()
                .all(|counter| counter.load(Ordering::Relaxed) == u64::MAX)
        );
    }
}
