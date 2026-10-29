//! Native atomic event staging for one completed local authorization attempt.

use super::*;
use crate::access_control::point_event_time::{next_event_time_from_frame, stamped_time_advances};
use crate::clock::ClockFrame;

/// One installed access event, with the state needed to report it after commit.
///
/// These snapshots preserve this attempt's order; they do not impose an ordering
/// on notification delivery between independent transactions.
pub struct CommittedAccessEvent {
    /// The installed report, with an actual timestamp. Absent credential/factor
    /// fields retain the existing no-credential/UNDEFINED representation.
    pub report: AccessEventReport,
    /// COV-readable intermediate state, independent of later point mutations.
    pub snapshot: Box<dyn BACnetObject>,
    /// Coherent Device-clock observation made while staging this event. An
    /// absent/invalid clock does not acquire a later timestamp during capture.
    pub clock_frame: Option<ClockFrame>,
}

impl AccessPointObject {
    pub(super) fn record_event_batch(
        &mut self,
        reports: &[AccessEventReport],
    ) -> Result<Vec<CommittedAccessEvent>, Error> {
        if self.out_of_service || self.reliability != Reliability::NO_FAULT_DETECTED {
            return Err(common::write_access_denied_error());
        }
        if reports.len() > 4 {
            return Err(common::value_out_of_range_error());
        }
        let mut staged = self.clone();
        let mut stamps = vec![self.access_event_time.clone()];
        let mut committed = Vec::with_capacity(reports.len());
        for input in reports {
            let mut report = input.clone();
            let frame = staged
                .clock
                .as_deref()
                .and_then(ClockReader::read_clock)
                .filter(|frame| frame.is_valid_actual_datetime());
            if report.time.is_none() {
                let next = next_event_time_from_frame(frame, &staged.access_event_time);
                if !stamped_time_advances(&staged.access_event_time, &next) {
                    return Err(common::value_out_of_range_error());
                }
                report.time = Some(next);
            }
            // Reuse the single-report reference/factor/event validation on the
            // detached state; a later failure cannot install a successful prefix.
            staged.set_access_event(report.clone())?;
            if stamps.contains(&staged.access_event_time) {
                return Err(common::value_out_of_range_error());
            }
            stamps.push(staged.access_event_time.clone());
            committed.push(CommittedAccessEvent {
                report,
                snapshot: Box::new(staged.clone()),
                clock_frame: frame,
            });
        }
        *self = staged;
        Ok(committed)
    }
}
