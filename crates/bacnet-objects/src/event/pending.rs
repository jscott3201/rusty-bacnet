//! A delayed transition's due instant (Clause 13.3), the delay toward a
//! target and the target under Event_Algorithm_Inhibit, shared by the
//! detectors.

use bacnet_time::{Duration, Instant};
use bacnet_types::enums::EventState;

/// Pending (delayed) intrinsic-reporting transition state, shared by every
/// detector that honors
/// [`OutOfRangeDetector::time_delay`](super::OutOfRangeDetector::time_delay)
/// and its peers.
///
/// Per ASHRAE 135-2020 Clause 13.3, a transition to a new `EventState` is
/// delayed by `Time_Delay` seconds, measured from when the condition began
/// to hold. While the delay runs the observable `event_state` stays at the
/// *old* (confirmed) state; if the triggering condition clears before the
/// delay elapses the pending transition is cancelled and no notification is
/// sent.
///
/// The transition is due at the instant the condition was first seen plus
/// the delay. The detector's periodic `tick` (for example
/// [`OutOfRangeDetector::tick`](super::OutOfRangeDetector::tick)) fires it
/// at the first call at or after that instant; `probe` only seeds or
/// cancels it, and a repeated write of the same value keeps the instant it
/// has, so a fast poll loop cannot shorten or restart the delay (#2303).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransition {
    /// The state the detector wants to transition to once the delay elapses.
    pub state: EventState,
    /// When the transition is due: when the condition was first seen plus
    /// the direction-appropriate delay (see `delay_toward`), on the host's
    /// monotonic timeline.
    pub due: Instant,
}

impl PendingTransition {
    /// Begin a pending transition to `state`, due `delay` seconds after
    /// `now`.
    pub(super) fn seed(state: EventState, delay: u32, now: Instant) -> Self {
        Self {
            state,
            due: now.saturating_add(Duration::from_secs(u64::from(delay))),
        }
    }
}

/// Select the delay governing a transition toward `target`.
///
/// ASHRAE 135-2020 Clause 13.3 gives the event algorithms two independent
/// delays. pTimeDelay measures the required persistence of an offnormal
/// condition in seconds before indicating that state: it governs
/// every indication into an OFFNORMAL state, including offnormal→offnormal
/// re-indication (CHANGE_OF_STATE (c) at 13.3.2, OUT_OF_RANGE (d)/(g) at
/// 13.3.6, COMMAND_FAILURE (a) at 13.3.4). pTimeDelayNormal measures the
/// required persistence of a Normal condition in seconds and gates only the
/// sustained-condition return to
/// NORMAL (CHANGE_OF_STATE (b), COMMAND_FAILURE (b), OUT_OF_RANGE (e)/(h)).
///
/// An absent pTimeDelayNormal uses pTimeDelay as its fallback, so `None`
/// behaves exactly as `time_delay`, never as an error or a zero.
///
/// FAULT never reaches this selector: Clause 13.2.2 fault precedence runs
/// ahead of the event algorithm and carries no delay term.
pub(super) fn delay_toward(
    time_delay: u32,
    time_delay_normal: Option<u32>,
    target: EventState,
) -> u32 {
    if target == EventState::NORMAL {
        time_delay_normal.unwrap_or(time_delay)
    } else {
        time_delay
    }
}

/// What an algorithm proposes while Event_Algorithm_Inhibit is TRUE, once
/// fault precedence has had its say (Clauses 13.2.2.1 and 13.2.2.1.5): its
/// own result is ignored and any pending transition dropped, so a condition has to
/// last its whole delay again once the inhibit clears, and an offnormal
/// state goes back to NORMAL at once. `None` when already NORMAL.
pub(super) fn inhibited_target(
    event_state: EventState,
    pending: &mut Option<PendingTransition>,
) -> Option<EventState> {
    *pending = None;
    (event_state != EventState::NORMAL).then_some(EventState::NORMAL)
}
