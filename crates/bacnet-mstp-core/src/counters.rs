//! Snapshot counters for diagnostics and interoperability tests.
//!
//! Counters are intentionally plain data.  The platform can copy a snapshot
//! without taking a lock, and all mutation helpers saturate rather than
//! wrapping when a long-running device exhausts a counter.

use crate::clock::Duration;

/// Counters for one bounded transmit queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueCounters {
    /// Current number of queued entries.
    pub depth: u32,
    /// Largest observed queue depth.
    pub high_water: u32,
    /// Number of entries accepted into the queue.
    pub enqueued: u32,
    /// Number of entries removed for transmission or expiry.
    pub dequeued: u32,
    /// Entries rejected because the queue was full.
    pub drops_full: u32,
    /// Entries rejected because their NPDU exceeded the storage capacity.
    pub drops_oversize: u32,
    /// Entries removed after exceeding their configured age.
    pub drops_expired: u32,
    /// Entries rejected or removed by starvation policy.
    pub drops_starved: u32,
}

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
        let depth = saturating_u32(depth);
        self.depth = depth;
        if depth > self.high_water {
            self.high_water = depth;
        }
    }
}

/// Observable MS/TP master-core counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MstpCounters {
    /// Number of state-machine transitions.
    pub state_transitions: u32,
    /// Deadlines observed after their scheduled instant.
    pub deadline_misses: u32,
    /// Tokens successfully rotated to a successor.
    pub token_rotations: u32,
    /// Token regeneration attempts after token loss.
    pub token_regenerations: u32,
    /// Token-pass retries.
    pub token_retries: u32,
    /// Parser or frame validation errors.
    pub parser_errors: u32,
    /// Frames abandoned by T_frame_abort.
    pub frame_aborts: u32,
    /// Valid MS/TP frames received.
    pub frames_received: u32,
    /// NPDUs delivered to the host/application.
    pub npdus_delivered: u32,
    /// Frames accepted for transmission and completed on the wire.
    pub frames_transmitted: u32,
    /// Actions dropped because the bounded action ring was full.
    pub action_drops: u32,
    /// Cumulative time spent with driver-enable asserted, in microseconds.
    pub de_assertion_us: u64,
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

    /// Record a missed or late deadline.
    pub(crate) fn record_deadline_miss(&mut self) {
        increment(&mut self.deadline_misses);
    }

    /// Record a successful token rotation.
    pub(crate) fn record_token_rotation(&mut self) {
        increment(&mut self.token_rotations);
    }

    /// Record token regeneration.
    pub(crate) fn record_token_regeneration(&mut self) {
        increment(&mut self.token_regenerations);
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

    /// Record an NPDU delivered to the host/application.
    pub(crate) fn record_npdu_delivered(&mut self) {
        increment(&mut self.npdus_delivered);
    }

    /// Record a frame whose final octet was transmitted on the wire.
    pub(crate) fn record_frame_transmitted(&mut self) {
        increment(&mut self.frames_transmitted);
    }

    /// Record an action-ring drop.
    pub(crate) fn record_action_drop(&mut self) {
        increment(&mut self.action_drops);
    }

    /// Add a completed DE assertion interval.
    pub(crate) fn record_de_assertion(&mut self, duration: Duration) {
        self.de_assertion_us = self.de_assertion_us.saturating_add(duration.as_micros());
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

fn saturating_u32(value: usize) -> u32 {
    if value > u32::MAX as usize {
        u32::MAX
    } else {
        value as u32
    }
}

#[cfg(test)]
mod tests {
    use super::MstpCounters;
    use crate::clock::Duration;

    #[test]
    fn counters_saturate_and_track_queue_depth() {
        let mut counters = MstpCounters::default();
        counters.record_transition();
        counters.record_frame_abort();
        counters.record_de_assertion(Duration::from_millis(3));
        assert_eq!(counters.state_transitions, 1);
        assert_eq!(counters.frame_aborts, 1);
        assert_eq!(counters.de_assertion_us, 3_000);

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
}
