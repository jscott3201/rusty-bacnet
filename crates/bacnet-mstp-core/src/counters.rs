//! What the core counts, drained by
//! [`MasterCore::take_counters`](crate::MasterCore::take_counters) into the
//! host's own totals.
//!
//! Each count runs from the last take and saturates at `u32::MAX` until the
//! next one, so a host that takes them often enough never loses one, and a
//! firmware image keeps no wider integers than it needs. The core keeps no
//! cumulative totals: the host adds each take to its own, as wide as it
//! needs (the Tokio MS/TP transport keeps `u64`s).

use bacnet_time::Duration;

use crate::actions::TransmitFailure;
use crate::state::ReceiveError;

/// Counters for one bounded transmit queue, since they were last taken
/// ([`QueueStorage::take_counters`](crate::QueueStorage::take_counters)).
///
/// The counts start again from zero at each take. [`Self::depth`] is a
/// level, not a count: the depth when they were read, which a take leaves
/// as it is, and [`Self::high_water`] the deepest the queue has been since
/// the last take, which a take sets back to the depth. The deepest ever is
/// then the largest high-water mark of the takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueCounters {
    /// Number of queued entries when the counters were read.
    pub depth: u32,
    /// Largest queue depth since the counters were last taken.
    pub high_water: u32,
    /// Number of entries accepted into the queue.
    pub enqueued: u32,
    /// Number of entries removed for transmission or expiry.
    pub dequeued: u32,
    /// Entries rejected because the queue was full.
    pub drops_full: u32,
    /// Entries refused or dropped as too long: refused at enqueue because the
    /// NPDU exceeds the queue's slot or a standard frame's 501 octets (the
    /// core's own check), or dropped at dequeue because it was longer than
    /// the caller takes.
    pub drops_oversize: u32,
    /// Entries removed after exceeding their configured age.
    pub drops_expired: u32,
}

// Only the queue backends record queue counters.
impl QueueCounters {
    /// Record an enqueue and the resulting depth.
    pub(crate) fn record_enqueue(&mut self, depth: usize) {
        increment(&mut self.enqueued);
        self.set_depth(depth);
    }

    /// Record a dequeue and the resulting depth.
    pub(crate) fn record_dequeue(&mut self, depth: usize) {
        increment(&mut self.dequeued);
        self.set_depth(depth);
    }

    /// Record a full-queue rejection.
    pub(crate) fn record_drop_full(&mut self) {
        increment(&mut self.drops_full);
    }

    /// Record an oversize-entry rejection.
    pub(crate) fn record_drop_oversize(&mut self) {
        increment(&mut self.drops_oversize);
    }

    /// Record an expiry.
    pub(crate) fn record_drop_expired(&mut self) {
        increment(&mut self.drops_expired);
    }

    /// Set current depth and update high-water mark.
    pub(crate) fn set_depth(&mut self, depth: usize) {
        let depth = u32::try_from(depth).unwrap_or(u32::MAX);
        self.depth = depth;
        if depth > self.high_water {
            self.high_water = depth;
        }
    }

    /// The counters, which start again: the counts from zero, the high-water
    /// mark from the depth, which stays.
    pub(crate) fn take(&mut self) -> Self {
        let taken = *self;
        *self = Self {
            depth: taken.depth,
            high_water: taken.depth,
            ..Self::default()
        };
        taken
    }
}

/// How late serviced deadlines were, in fixed buckets, and the latest.
///
/// A bucket holds the lateness from its lower edge, inclusive, to the next
/// edge, exclusive: under 1 ms, 1 to 2 ms, 2 to 5 ms, 5 to 10 ms, 10 to
/// 25 ms, and 25 ms or more ([`Self::EDGES`]). A lateness of exactly 5 ms
/// therefore counts in the 5 to 10 ms bucket. Each bucket saturates at
/// `u32::MAX` between takes, as the other counts do; [`Self::max`] is the
/// largest lateness since the last take.
///
/// It needs no allocation and no division: a lateness finds its bucket by
/// comparison with the edges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LatenessHistogram {
    /// Deadlines serviced in each lateness band, [`Self::EDGES`] apart.
    pub buckets: [u32; LATENESS_BUCKETS],
    /// The largest lateness of a serviced deadline.
    pub max: Duration,
}

/// The number of buckets in a [`LatenessHistogram`].
pub const LATENESS_BUCKETS: usize = 6;

impl LatenessHistogram {
    /// The buckets' edges: each one closes a bucket and opens the next. The
    /// first bucket starts at zero and the last has no upper edge.
    ///
    /// The bands keep apart a host timer's rounding (under 2 ms), ordinary
    /// scheduling jitter (2 to 5 ms), lateness that eats into a Clause 9.5.3
    /// window such as T_slot (5 to 10 ms), and lateness past one (10 ms or
    /// more).
    pub const EDGES: [Duration; LATENESS_BUCKETS - 1] = [
        Duration::from_millis(1),
        Duration::from_millis(2),
        Duration::from_millis(5),
        Duration::from_millis(10),
        Duration::from_millis(25),
    ];

    /// The bucket a lateness counts in: the number of edges at or below it.
    #[must_use]
    pub fn bucket(lateness: Duration) -> usize {
        Self::EDGES.iter().filter(|&&edge| lateness >= edge).count()
    }

    /// Deadlines counted, all buckets together, saturating.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.buckets
            .iter()
            .fold(0u32, |sum, &count| sum.saturating_add(count))
    }

    /// Count one serviced deadline.
    pub(crate) fn record(&mut self, lateness: Duration) {
        increment(&mut self.buckets[Self::bucket(lateness)]);
        self.max = self.max.max(lateness);
    }
}

/// Everything the master core counts, since the counters were last taken
/// ([`MasterCore::take_counters`](crate::MasterCore::take_counters)).
///
/// Each count saturates at `u32::MAX` between takes; add each take to your
/// own totals. [`Self::de_assertion`] is a time, summed the same way, and
/// the queue counters' depth is a level (see [`QueueCounters`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpCounters {
    /// Number of state-machine transitions.
    pub state_transitions: u32,
    /// Deadlines serviced [`MstpCoreConfig::late_deadline`] or more after
    /// they fell due: the late tail of [`Self::deadline_lateness`].
    ///
    /// [`MstpCoreConfig::late_deadline`]: crate::MstpCoreConfig::late_deadline
    pub late_deadlines: u32,
    /// How late every serviced deadline was, early wakes aside: one count
    /// per timed wake, so its total is the number of deadlines serviced.
    pub deadline_lateness: LatenessHistogram,
    /// Tokens successfully rotated to a successor.
    pub token_rotations: u32,
    /// Token regeneration attempts after token loss.
    pub token_regenerations: u32,
    /// Tokens generated in a repeated window rather than in this station's
    /// first slot of the silence, because the NO_TOKEN deadline was serviced
    /// after that slot closed. A local recovery policy, not Clause 9.5.6.7
    /// behaviour; see [`MasterCore`](crate::MasterCore).
    pub missed_generation_slots: u32,
    /// Token-pass retries.
    pub token_retries: u32,
    /// Received frames counted invalid: bad headers, and frames for this
    /// station or broadcast that were too long, undecodable or corrupt.
    pub parser_errors: u32,
    /// Answers whose transmission started after their send-by time:
    /// T_reply_delay for a data reply, Reply Postponed or Test_Response, and
    /// T_usage_delay for a Reply To Poll For Master. They were still sent.
    pub late_replies: u32,
    /// Answers dropped unsent because the asker may already have stopped
    /// waiting: its shortest T_reply_timeout (255 ms), or for a Reply To Poll
    /// For Master its shortest T_usage_timeout (20 ms), less one octet time
    /// and a 1 ms guard. An answer aborted before it started counts here
    /// too, whether the adapter aborted it or the core's transmit watchdog
    /// did because the adapter never started it: its request goes
    /// unanswered.
    pub expired_requests: u32,
    /// Frames other than answers given up before they went out: line
    /// activity kept them waiting past their limit, or was the evidence
    /// their state watches for (SawTokenUser, SawFrame, a late Reply To Poll
    /// For Master), or they were aborted before they started, by the adapter
    /// or by the core's transmit watchdog. Line activity alone only delays a
    /// frame.
    pub cancelled_sends: u32,
    /// Transmit starts refused because the line was active within
    /// T_turnaround of them (Clause 9.5.5.1). Each frame waited for a quiet
    /// line again.
    pub deferred_starts: u32,
    /// Inputs ignored or refused because the adapter broke the core's
    /// contract: starting a transmit before polling it, confirming a start
    /// twice, completing a transmit it never confirmed starting, or handing
    /// [`MasterCore::on_validated_frame`](crate::MasterCore::on_validated_frame)
    /// a frame the codec would not have decoded.
    pub adapter_errors: u32,
    /// Broken internal invariants: a frame the core built that its own frame
    /// buffer could not hold. Debug builds assert instead; a release build
    /// counts it here and does not send the frame. Zero unless the core has
    /// a bug.
    pub internal_errors: u32,
    /// Frames for this station that it could not act on, so dropped: a Poll
    /// For Master, data request, Test_Request or Reply To Poll For Master
    /// that arrived while it was answering a request or waiting to send a
    /// frame of its own, or a Token that arrived while a frame it sends as
    /// token holder waited.
    pub discarded_while_busy: u32,
    /// Frames abandoned part way through: by T_frame_abort, or when this
    /// station confirmed a transmit start, since its receiver is off while
    /// its driver is on.
    pub frame_aborts: u32,
    /// Valid MS/TP frames received.
    pub frames_received: u32,
    /// Valid frames received whose source address is this station's, outside
    /// this station's own transmissions, whose echo is ignored. Each is
    /// evidence that another station uses the same address. Clause 9 gives
    /// such a frame no rule of its own, so it is otherwise handled as usual.
    pub own_address_frames: u32,
    /// NPDUs delivered to the host/application.
    pub npdus_delivered: u32,
    /// Frames delivered whole to the host: Test_Response and proprietary
    /// frames ([`Action::DeliverFrame`](crate::Action::DeliverFrame)).
    pub frames_delivered: u32,
    /// Frames accepted for transmission and completed on the wire.
    pub frames_transmitted: u32,
    /// Deliveries dropped because every delivery slot of the core's
    /// [`ActionBuffers`](crate::ActionBuffers) was full. Control actions
    /// are never dropped.
    pub delivery_drops: u32,
    /// Receive errors reported as [`ReceiveError::Framing`], outside this
    /// station's own transmissions (whose echo is ignored).
    pub framing_errors: u32,
    /// Receive errors reported as [`ReceiveError::Overrun`], the same way.
    pub overrun_errors: u32,
    /// Receive errors reported as [`ReceiveError::Parity`], the same way.
    pub parity_errors: u32,
    /// Receive errors reported as [`ReceiveError::Break`], the same way.
    pub break_errors: u32,
    /// Transmits aborted as [`TransmitFailure::Aborted`].
    pub transmit_aborts: u32,
    /// Transmits aborted as [`TransmitFailure::Timeout`], by the adapter or
    /// by the core's own transmit watchdog.
    pub transmit_timeouts: u32,
    /// Transmits aborted as [`TransmitFailure::Io`].
    pub transmit_io_errors: u32,
    /// Transmits aborted as [`TransmitFailure::Rejected`].
    pub transmit_rejections: u32,
    /// Time spent with driver-enable asserted.
    pub de_assertion: Duration,
    /// Unicast queue counters.
    pub unicast: QueueCounters,
    /// Broadcast queue counters.
    pub broadcast: QueueCounters,
}

impl MstpCounters {
    /// Record a state transition.
    pub(crate) fn record_transition(&mut self) {
        increment(&mut self.state_transitions);
    }

    /// Record a serviced deadline, `lateness` after it fell due, and count
    /// it late at `late_deadline` or more.
    pub(crate) fn record_deadline(&mut self, lateness: Duration, late_deadline: Duration) {
        self.deadline_lateness.record(lateness);
        if lateness >= late_deadline {
            increment(&mut self.late_deadlines);
        }
    }

    /// Record a reply that started after T_reply_delay.
    pub(crate) fn record_late_reply(&mut self) {
        increment(&mut self.late_replies);
    }

    /// Record a request dropped unanswered.
    pub(crate) fn record_expired_request(&mut self) {
        increment(&mut self.expired_requests);
    }

    /// Record a frame given up before it went out.
    pub(crate) fn record_cancelled_send(&mut self) {
        increment(&mut self.cancelled_sends);
    }

    /// Record a transmit start refused for a broken turnaround.
    pub(crate) fn record_deferred_start(&mut self) {
        increment(&mut self.deferred_starts);
    }

    /// Record a received frame dropped because the station was busy.
    pub(crate) fn record_discarded_while_busy(&mut self) {
        increment(&mut self.discarded_while_busy);
    }

    /// Record an input ignored or refused for breaking the core's contract.
    pub(crate) fn record_adapter_error(&mut self) {
        increment(&mut self.adapter_errors);
    }

    /// Record a broken internal invariant.
    pub(crate) fn record_internal_error(&mut self) {
        increment(&mut self.internal_errors);
    }

    /// Record a successful token rotation.
    pub(crate) fn record_token_rotation(&mut self) {
        increment(&mut self.token_rotations);
    }

    /// Record token regeneration.
    pub(crate) fn record_token_regeneration(&mut self) {
        increment(&mut self.token_regenerations);
    }

    /// Record a token generated in a repeated window.
    pub(crate) fn record_missed_generation_slot(&mut self) {
        increment(&mut self.missed_generation_slots);
    }

    /// Record a token retry.
    pub(crate) fn record_token_retry(&mut self) {
        increment(&mut self.token_retries);
    }

    /// Record a parser error.
    pub(crate) fn record_parser_error(&mut self) {
        increment(&mut self.parser_errors);
    }

    /// Record a frame aborted by the inter-octet timeout.
    pub(crate) fn record_frame_abort(&mut self) {
        increment(&mut self.frame_aborts);
    }

    /// Record a valid received frame.
    pub(crate) fn record_frame_received(&mut self) {
        increment(&mut self.frames_received);
    }

    /// Record a received frame from this station's address.
    pub(crate) fn record_own_address_frame(&mut self) {
        increment(&mut self.own_address_frames);
    }

    /// Record an NPDU delivered to the host/application.
    pub(crate) fn record_npdu_delivered(&mut self) {
        increment(&mut self.npdus_delivered);
    }

    /// Record a frame delivered whole to the host.
    pub(crate) fn record_frame_delivered(&mut self) {
        increment(&mut self.frames_delivered);
    }

    /// Record a frame whose final octet was transmitted on the wire.
    pub(crate) fn record_frame_transmitted(&mut self) {
        increment(&mut self.frames_transmitted);
    }

    /// Record a delivery dropped for want of a slot.
    pub(crate) fn record_delivery_drop(&mut self) {
        increment(&mut self.delivery_drops);
    }

    /// Add a completed DE assertion interval.
    pub(crate) fn record_de_assertion(&mut self, duration: Duration) {
        self.de_assertion = self.de_assertion.saturating_add(duration);
    }

    /// Record a receive error, by kind.
    pub(crate) fn record_receive_error(&mut self, error: ReceiveError) {
        increment(match error {
            ReceiveError::Framing => &mut self.framing_errors,
            ReceiveError::Overrun => &mut self.overrun_errors,
            ReceiveError::Parity => &mut self.parity_errors,
            ReceiveError::Break => &mut self.break_errors,
        });
    }

    /// Record an aborted transmit, by reason.
    pub(crate) fn record_transmit_failure(&mut self, reason: TransmitFailure) {
        increment(match reason {
            TransmitFailure::Aborted => &mut self.transmit_aborts,
            TransmitFailure::Timeout => &mut self.transmit_timeouts,
            TransmitFailure::Io => &mut self.transmit_io_errors,
            TransmitFailure::Rejected => &mut self.transmit_rejections,
        });
    }

    /// Return the queue selected by the broadcast flag.
    #[cfg(test)]
    pub(crate) fn queue_mut(&mut self, broadcast: bool) -> &mut QueueCounters {
        if broadcast {
            &mut self.broadcast
        } else {
            &mut self.unicast
        }
    }
}

fn increment(value: &mut u32) {
    *value = value.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::{LatenessHistogram, MstpCounters, QueueCounters};
    use crate::{ReceiveError, TransmitFailure};
    use bacnet_time::Duration;

    #[test]
    fn counters_saturate_and_track_queue_depth() {
        let mut counters = MstpCounters::default();
        counters.record_transition();
        counters.record_frame_abort();
        counters.record_de_assertion(Duration::from_millis(3));
        assert_eq!(counters.state_transitions, 1);
        assert_eq!(counters.frame_aborts, 1);
        assert_eq!(counters.de_assertion, Duration::from_millis(3));
        // Intervals add up, to the nanosecond, and stop at the maximum.
        counters.record_de_assertion(Duration::from_nanos(1_500));
        assert_eq!(counters.de_assertion, Duration::from_nanos(3_001_500));
        counters.record_de_assertion(Duration::MAX);
        assert_eq!(counters.de_assertion, Duration::MAX);

        counters.unicast.record_enqueue(2);
        counters.unicast.record_enqueue(4);
        counters.unicast.record_dequeue(3);
        counters.unicast.record_drop_full();
        assert_eq!(counters.unicast.depth, 3);
        assert_eq!(counters.unicast.high_water, 4);
        assert_eq!(counters.unicast.drops_full, 1);
    }

    #[test]
    fn queue_selection_is_per_class() {
        let mut counters = MstpCounters::default();
        counters.queue_mut(false).record_drop_oversize();
        counters.queue_mut(true).record_drop_expired();
        assert_eq!(counters.unicast.drops_oversize, 1);
        assert_eq!(counters.broadcast.drops_expired, 1);
    }

    /// A take returns the counts and starts them again from zero; the depth
    /// stays, and the high-water mark starts again from it.
    #[test]
    fn a_take_starts_the_counts_again_and_keeps_the_depth() {
        let mut queue = QueueCounters::default();
        queue.record_enqueue(1);
        queue.record_enqueue(2);
        queue.record_enqueue(3);
        queue.record_dequeue(2);
        queue.record_drop_full();
        let taken = queue.take();
        assert_eq!(
            (
                taken.depth,
                taken.high_water,
                taken.enqueued,
                taken.dequeued
            ),
            (2, 3, 3, 1)
        );
        assert_eq!(taken.drops_full, 1);
        let mut rest = QueueCounters::default();
        (rest.depth, rest.high_water) = (2, 2);
        assert_eq!(queue, rest);
        queue.record_dequeue(1);
        assert_eq!(queue.take().high_water, 2);
        assert_eq!((queue.depth, queue.high_water), (1, 1));
    }

    /// Each edge opens its bucket: a lateness exactly at an edge counts in
    /// the bucket above it, and a nanosecond less in the one below.
    #[test]
    fn lateness_buckets_split_at_their_edges() {
        let ms = Duration::from_millis;
        let below = |edge: Duration| edge - Duration::from_nanos(1);
        for (lateness, bucket) in [
            (Duration::ZERO, 0),
            (below(ms(1)), 0),
            (ms(1), 1),
            (below(ms(2)), 1),
            (ms(2), 2),
            (below(ms(5)), 2),
            (ms(5), 3),
            (below(ms(10)), 3),
            (ms(10), 4),
            (below(ms(25)), 4),
            (ms(25), 5),
            (Duration::MAX, 5),
        ] {
            assert_eq!(LatenessHistogram::bucket(lateness), bucket, "{lateness:?}");
        }
    }

    /// Every serviced deadline counts in one bucket, so the total is the
    /// number serviced; the maximum is the largest; the late count takes
    /// the threshold as inclusive; and a take starts them all again.
    #[test]
    fn every_deadline_counts_once_and_the_threshold_is_inclusive() {
        let ms = Duration::from_millis;
        let mut counters = MstpCounters::default();
        for lateness in [ms(0), ms(1), ms(3), ms(5), ms(7), ms(4)] {
            counters.record_deadline(lateness, ms(5));
        }
        let lateness = counters.deadline_lateness;
        assert_eq!(lateness.buckets, [1, 1, 2, 2, 0, 0]);
        assert_eq!(lateness.total(), 6);
        assert_eq!(lateness.max, ms(7));
        // 5 ms and 7 ms reach the 5 ms threshold; 4 ms doesn't.
        assert_eq!(counters.late_deadlines, 2);
        // A zero threshold counts every deadline late.
        counters.record_deadline(Duration::ZERO, Duration::ZERO);
        assert_eq!(counters.late_deadlines, 3);
        // Buckets saturate like the other counts.
        counters.deadline_lateness.buckets[0] = u32::MAX;
        counters.record_deadline(Duration::ZERO, ms(5));
        assert_eq!(counters.deadline_lateness.buckets[0], u32::MAX);
        assert_eq!(counters.deadline_lateness.total(), u32::MAX);
    }

    /// Each receive error and transmit failure counts under its own kind.
    #[test]
    fn errors_and_failures_count_by_kind() {
        let mut counters = MstpCounters::default();
        for error in [
            ReceiveError::Framing,
            ReceiveError::Overrun,
            ReceiveError::Overrun,
            ReceiveError::Parity,
            ReceiveError::Break,
        ] {
            counters.record_receive_error(error);
        }
        for reason in [
            TransmitFailure::Aborted,
            TransmitFailure::Timeout,
            TransmitFailure::Io,
            TransmitFailure::Io,
            TransmitFailure::Rejected,
        ] {
            counters.record_transmit_failure(reason);
        }
        assert_eq!(
            [
                counters.framing_errors,
                counters.overrun_errors,
                counters.parity_errors,
                counters.break_errors,
            ],
            [1, 2, 1, 1]
        );
        assert_eq!(
            [
                counters.transmit_aborts,
                counters.transmit_timeouts,
                counters.transmit_io_errors,
                counters.transmit_rejections,
            ],
            [1, 1, 2, 1]
        );
    }
}
