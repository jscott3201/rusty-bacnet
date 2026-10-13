//! The master core's counters, added up for the life of a transport.
//!
//! The core counts in `u32`s since its counters were last taken
//! ([`MasterCore::take_counters`](bacnet_mstp_core::MasterCore::take_counters)),
//! and keeps no totals. The transport's task takes them after every event it
//! handles, and a send call takes its queue's, and each take is added here,
//! into `u64`s that saturate.

use bacnet_mstp_core::{
    Duration, MstpCounters as CoreCounters, QueueCounters as CoreQueueCounters,
};

/// One transmit queue's counts since the transport started: the core's
/// [`QueueCounters`](bacnet_mstp_core::QueueCounters), added up. Each count
/// saturates at `u64::MAX`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueCounters {
    /// Entries queued when the counters were last published.
    pub depth: u64,
    /// The most entries the queue has held.
    pub high_water: u64,
    /// Entries accepted into the queue.
    pub enqueued: u64,
    /// Entries removed, for transmission or at their age.
    pub dequeued: u64,
    /// Entries refused because the queue was full.
    pub drops_full: u64,
    /// Entries refused or dropped as too long to send.
    pub drops_oversize: u64,
    /// Entries removed at their maximum age.
    pub drops_expired: u64,
}

impl QueueCounters {
    /// Add one take: its counts, the depth it saw, and its high-water mark
    /// if that is higher.
    pub(super) fn add(&mut self, taken: CoreQueueCounters) {
        self.depth = u64::from(taken.depth);
        self.high_water = self.high_water.max(u64::from(taken.high_water));
        add(&mut self.enqueued, taken.enqueued);
        add(&mut self.dequeued, taken.dequeued);
        add(&mut self.drops_full, taken.drops_full);
        add(&mut self.drops_oversize, taken.drops_oversize);
        add(&mut self.drops_expired, taken.drops_expired);
    }
}

/// The master core's counts since the transport started: its
/// [`MstpCounters`](bacnet_mstp_core::MstpCounters), added up, where each
/// field is described. Each count saturates at `u64::MAX`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpCounters {
    /// State-machine transitions.
    pub state_transitions: u64,
    /// Deadlines serviced a millisecond or more after they fell due.
    pub deadline_misses: u64,
    /// Tokens passed to a successor.
    pub token_rotations: u64,
    /// Tokens generated after the token was lost.
    pub token_regenerations: u64,
    /// Tokens generated in a later window than this station's first slot.
    pub missed_generation_slots: u64,
    /// Token-pass retries.
    pub token_retries: u64,
    /// Received frames counted invalid.
    pub parser_errors: u64,
    /// Answers that started after their send-by time, sent anyway.
    pub late_replies: u64,
    /// Answers dropped unsent because the asker may have stopped waiting.
    pub expired_requests: u64,
    /// Frames other than answers given up before they went out.
    pub cancelled_sends: u64,
    /// Transmit starts refused because the line was active within
    /// T_turnaround.
    pub deferred_starts: u64,
    /// Transmit events refused because they broke the transmit contract.
    pub adapter_errors: u64,
    /// Frames for this station dropped because it was busy.
    pub discarded_while_busy: u64,
    /// Frames abandoned part way through.
    pub frame_aborts: u64,
    /// Valid frames received.
    pub frames_received: u64,
    /// Valid frames received from this station's own address, outside its
    /// own transmissions.
    pub own_address_frames: u64,
    /// NPDUs delivered.
    pub npdus_delivered: u64,
    /// Frames delivered whole: Test_Response and proprietary frames.
    pub frames_delivered: u64,
    /// Frames sent to completion.
    pub frames_transmitted: u64,
    /// Deliveries dropped because every delivery slot was full.
    pub delivery_drops: u64,
    /// Framing errors, outside this station's own transmissions. This
    /// transport reports no receive errors to the core (a host serial API
    /// gives none per octet), so this and the three below stay zero.
    pub framing_errors: u64,
    /// Overruns, the same way; zero here.
    pub overrun_errors: u64,
    /// Parity errors, the same way; zero here.
    pub parity_errors: u64,
    /// Breaks, the same way; zero here.
    pub break_errors: u64,
    /// Transmits aborted as `Aborted`. This transport never aborts with that
    /// reason, so it stays zero.
    pub transmit_aborts: u64,
    /// Transmits aborted as `Timeout`: here only by the core's own transmit
    /// watchdog, since the transport never reports a timeout itself.
    pub transmit_timeouts: u64,
    /// Transmits aborted as `Io`: a serial write or drain failed, the only
    /// reason this transport reports.
    pub transmit_io_errors: u64,
    /// Transmits aborted as `Rejected`. This transport never aborts with
    /// that reason, so it stays zero.
    pub transmit_rejections: u64,
    /// Time spent with driver-enable asserted.
    pub de_assertion: Duration,
    /// The unicast queue's counts.
    pub unicast: QueueCounters,
    /// The broadcast queue's counts.
    pub broadcast: QueueCounters,
}

impl MstpCounters {
    /// Add one take of the core's counters.
    pub(super) fn add(&mut self, taken: CoreCounters) {
        add(&mut self.state_transitions, taken.state_transitions);
        add(&mut self.deadline_misses, taken.deadline_misses);
        add(&mut self.token_rotations, taken.token_rotations);
        add(&mut self.token_regenerations, taken.token_regenerations);
        add(
            &mut self.missed_generation_slots,
            taken.missed_generation_slots,
        );
        add(&mut self.token_retries, taken.token_retries);
        add(&mut self.parser_errors, taken.parser_errors);
        add(&mut self.late_replies, taken.late_replies);
        add(&mut self.expired_requests, taken.expired_requests);
        add(&mut self.cancelled_sends, taken.cancelled_sends);
        add(&mut self.deferred_starts, taken.deferred_starts);
        add(&mut self.adapter_errors, taken.adapter_errors);
        add(&mut self.discarded_while_busy, taken.discarded_while_busy);
        add(&mut self.frame_aborts, taken.frame_aborts);
        add(&mut self.frames_received, taken.frames_received);
        add(&mut self.own_address_frames, taken.own_address_frames);
        add(&mut self.npdus_delivered, taken.npdus_delivered);
        add(&mut self.frames_delivered, taken.frames_delivered);
        add(&mut self.frames_transmitted, taken.frames_transmitted);
        add(&mut self.delivery_drops, taken.delivery_drops);
        add(&mut self.framing_errors, taken.framing_errors);
        add(&mut self.overrun_errors, taken.overrun_errors);
        add(&mut self.parity_errors, taken.parity_errors);
        add(&mut self.break_errors, taken.break_errors);
        add(&mut self.transmit_aborts, taken.transmit_aborts);
        add(&mut self.transmit_timeouts, taken.transmit_timeouts);
        add(&mut self.transmit_io_errors, taken.transmit_io_errors);
        add(&mut self.transmit_rejections, taken.transmit_rejections);
        self.de_assertion = self.de_assertion.saturating_add(taken.de_assertion);
        self.unicast.add(taken.unicast);
        self.broadcast.add(taken.broadcast);
    }
}

fn add(total: &mut u64, count: u32) {
    *total = total.saturating_add(u64::from(count));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names of the fields a `{:#?}` shows at its top level.
    fn fields(debug: &str) -> Vec<String> {
        debug
            .lines()
            .filter_map(|line| line.strip_prefix("    "))
            .filter(|line| !line.starts_with(' '))
            .filter_map(|line| line.split_once(':').map(|(name, _)| name.to_string()))
            .collect()
    }

    /// Every core counter has a total of the same name, so none is left
    /// out of `add` when the core gains one (its counters are
    /// `#[non_exhaustive]`, so no exhaustive pattern can check it).
    #[test]
    fn every_core_counter_has_a_total() {
        let core = fields(&format!("{:#?}", CoreCounters::default()));
        assert!(core.len() > 30, "{core:?}");
        assert_eq!(core, fields(&format!("{:#?}", MstpCounters::default())));
        assert_eq!(
            fields(&format!("{:#?}", CoreQueueCounters::default())),
            fields(&format!("{:#?}", QueueCounters::default()))
        );
    }

    /// One take, every field of it a different value, adds each field to
    /// the total of the same name and no other: the totals then read as
    /// the take, field by field (a `u32` and a `u64` print alike).
    #[test]
    fn each_count_goes_to_its_own_total() {
        let mut taken = CoreCounters::default();
        let counts = [
            &mut taken.state_transitions,
            &mut taken.deadline_misses,
            &mut taken.token_rotations,
            &mut taken.token_regenerations,
            &mut taken.missed_generation_slots,
            &mut taken.token_retries,
            &mut taken.parser_errors,
            &mut taken.late_replies,
            &mut taken.expired_requests,
            &mut taken.cancelled_sends,
            &mut taken.deferred_starts,
            &mut taken.adapter_errors,
            &mut taken.discarded_while_busy,
            &mut taken.frame_aborts,
            &mut taken.frames_received,
            &mut taken.own_address_frames,
            &mut taken.npdus_delivered,
            &mut taken.frames_delivered,
            &mut taken.frames_transmitted,
            &mut taken.delivery_drops,
            &mut taken.framing_errors,
            &mut taken.overrun_errors,
            &mut taken.parity_errors,
            &mut taken.break_errors,
            &mut taken.transmit_aborts,
            &mut taken.transmit_timeouts,
            &mut taken.transmit_io_errors,
            &mut taken.transmit_rejections,
        ];
        for (value, count) in (1..).zip(counts) {
            *count = value;
        }
        taken.de_assertion = Duration::from_nanos(1_234_567);
        for (base, queue) in [(100, &mut taken.unicast), (200, &mut taken.broadcast)] {
            queue.depth = base + 1;
            queue.high_water = base + 2;
            queue.enqueued = base + 3;
            queue.dequeued = base + 4;
            queue.drops_full = base + 5;
            queue.drops_oversize = base + 6;
            queue.drops_expired = base + 7;
        }
        let mut totals = MstpCounters::default();
        totals.add(taken);
        assert_eq!(format!("{totals:#?}"), format!("{taken:#?}"));
    }

    /// Each take adds every count, past `u32::MAX`; the depth is the
    /// latest take's and the high-water mark the highest.
    #[test]
    fn takes_add_up_past_u32() {
        let mut taken = CoreCounters::default();
        taken.frames_received = u32::MAX;
        taken.transmit_io_errors = 2;
        taken.de_assertion = Duration::from_millis(3);
        taken.unicast.enqueued = u32::MAX;
        (taken.unicast.depth, taken.unicast.high_water) = (5, 9);
        let mut totals = MstpCounters::default();
        totals.add(taken);
        (taken.unicast.depth, taken.unicast.high_water) = (2, 4);
        totals.add(taken);
        assert_eq!(totals.frames_received, 2 * u64::from(u32::MAX));
        assert_eq!(totals.transmit_io_errors, 4);
        assert_eq!(totals.de_assertion, Duration::from_millis(6));
        assert_eq!(totals.unicast.enqueued, 2 * u64::from(u32::MAX));
        assert_eq!((totals.unicast.depth, totals.unicast.high_water), (2, 9));
        totals.frames_received = u64::MAX - 1;
        totals.add(taken);
        assert_eq!(totals.frames_received, u64::MAX);
    }
}
