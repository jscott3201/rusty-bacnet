//! The event-reporting capability: intrinsic reporting's proposals and
//! commits, alarm acknowledgment and Event_Algorithm_Inhibit_Ref following.

use bacnet_time::Instant;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState};
use bacnet_types::error::Error;
use bacnet_types::primitives::BACnetTimeStamp;

use crate::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use crate::log_reporting::BufferReadyReport;

/// OBJECT / NO_ALARM_CONFIGURED: what an object answers for an
/// acknowledgment it can't take, such as one with event detection disabled,
/// and what a host answers for an object that lends no [`EventReporting`].
pub fn no_alarm_configured() -> Error {
    Error::protocol(ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED)
}

/// An object that reports events: an intrinsic reporter, an Event or Alert
/// Enrollment, or a log that reports BUFFER_READY.
///
/// The object returns itself from
/// [`BACnetObject::event_reporting_mut`](crate::traits::BACnetObject::event_reporting_mut),
/// and from [`event_reporting`](crate::traits::BACnetObject::event_reporting)
/// for the queries a host makes under a shared database guard, such as
/// GetEnrollmentSummary and a BUFFER_READY notification's counts. An object
/// that lends one lends the other.
///
/// Every method has a default that opts out, so an object overrides only
/// what it models. Proposals and commits are separate steps: an evaluation
/// only proposes a transition, and the object's event properties change
/// when the host commits it through
/// [`commit_event_transition`](Self::commit_event_transition). The bundled
/// server runs the evaluation after a write, and the periodic tick once a
/// second and whenever [`next_due`](Self::next_due) says a delay ends; it
/// commits each proposal while it holds the database's write guard, and
/// sends the notifications only after it has released that guard. Every
/// `now` is the host's monotonic instant (`bacnet_time::Instant`), one
/// timeline for every object.
pub trait EventReporting {
    /// Evaluate intrinsic reporting after a Present_Value change.
    ///
    /// This is the per-write entry point: it seeds (or cancels) a pending
    /// delayed transition, due `Time_Delay` seconds after `now` (Clause
    /// 13.3), and fires immediately only when `Time_Delay == 0`. It never
    /// fires a pending transition, and a repeated write of the same value
    /// keeps the due instant it has:
    /// [`tick_intrinsic_reporting`](Self::tick_intrinsic_reporting) fires it.
    ///
    /// Returns `Some(TransitionOutcome)` whenever a transition is ready to be
    /// committed, or `None` when none is (no change, a delay seeded, or no
    /// intrinsic reporting, the default). A cleared `Event_Enable` bit sets
    /// the outcome's `distribute` flag to false rather than withholding the
    /// transition. Implementations must leave `Event_State`,
    /// `Acked_Transitions`, event history and fire-ready detector state
    /// unchanged until [`commit_event_transition`](Self::commit_event_transition)
    /// succeeds. Clause 13.2.2.1.4's transition actions run either way, and
    /// `Event_Enable` disables only external distribution, downstream in the
    /// notification-distribution process (Clause 13.2.5).
    fn evaluate_intrinsic_reporting(&mut self, _now: Instant) -> Option<TransitionOutcome> {
        None
    }

    /// The periodic evaluation: fire a pending transition that is due.
    ///
    /// Called by the server's intrinsic-reporting task. Fires the pending
    /// transition when its due instant is at or before `now`, cancels it if
    /// the triggering condition reverted, seeds one for a condition no write
    /// reported (due its delay after `now`), and returns
    /// `Some(TransitionOutcome)` when a transition is ready. A fire-ready
    /// proposal must remain retryable until the commit succeeds. As with
    /// [`evaluate_intrinsic_reporting`](Self::evaluate_intrinsic_reporting),
    /// `Event_Enable` is reported via `distribute`, not by returning `None`.
    /// Objects without a delayed transition return `None`, the default.
    fn tick_intrinsic_reporting(&mut self, _now: Instant) -> Option<TransitionOutcome> {
        None
    }

    /// When this object's pending transition is due, if one is: the latest
    /// instant a host should call
    /// [`tick_intrinsic_reporting`](Self::tick_intrinsic_reporting) by for
    /// the delay to end on time. `None` with nothing pending, the default.
    fn next_due(&self) -> Option<Instant> {
        None
    }

    /// Atomically commit all object-owned state for one event transition.
    ///
    /// The caller supplies an exact state change, its transition coordinate,
    /// the resolved Notification Class `Ack_Required` value, a typed
    /// timestamp, and an optional message. Implementations must validate the
    /// coordinate and source state before changing `Event_State`,
    /// `Acked_Transitions`, `Event_Time_Stamps`, or stored message state, and
    /// must leave every value unchanged on error. A successful
    /// implementation also finalizes the detector's pending and fault-edge
    /// state; a failed commit leaves that state retryable.
    ///
    /// While the object's `Event_Detection_Enable` is FALSE, the state
    /// machine is stopped and the event properties stay at their initial
    /// values (Clause 13.2.2.1), so no commit lands, NORMAL targets
    /// included. The built-in reporters refuse every such commit with
    /// [`EventTransitionCommitError::DetectionDisabled`], changing nothing,
    /// and an implementation with that property should do the same; turning
    /// detection off resets the event properties directly, never through a
    /// commit. Two built-in exceptions answer first with
    /// [`EventTransitionCommitError::Unsupported`]: the logs (Trend Log,
    /// Trend Log Multiple, Event Log) for any target other than NORMAL,
    /// detection on or off, and Alert Enrollment, which has no commit.
    ///
    /// This channel is deliberately separate from network property writes
    /// and notification distribution. The default fails closed with
    /// [`EventTransitionCommitError::Unsupported`], so an object takes part
    /// only once it implements this contract; it owns and commits its state
    /// directly, with no built-in commit kernel required. Both server
    /// intrinsic paths require success before distributing a notification.
    fn commit_event_transition(
        &mut self,
        _commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        Err(EventTransitionCommitError::Unsupported)
    }

    /// Acknowledge the committed transition an AcknowledgeAlarm request
    /// names by its event state and its timestamp, setting that
    /// transition's `Acked_Transitions` bit.
    ///
    /// Returns the transition's exact from and to states when the object
    /// keeps them, or `Ok(None)` when it doesn't; either way the
    /// acknowledgment was taken. The bundled server sends an acknowledgment
    /// notification only when it has those states and also knows the
    /// object's event algorithm, which a custom object can't yet supply
    /// (#2179); until then a custom object's accepted acknowledgment sends
    /// none.
    /// A request that matches no committed transition fails with the
    /// service error it earns (INVALID_EVENT_STATE or INVALID_TIMESTAMP),
    /// and an object that can't take acknowledgments, the default, with
    /// [`no_alarm_configured`]. Implementations leave `Acked_Transitions`
    /// unchanged on error.
    fn acknowledge_alarm(
        &mut self,
        _event_state: EventState,
        _timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        Err(no_alarm_configured())
    }

    /// This object's GetEnrollmentSummary event capability: its one event
    /// algorithm and the coordinate of its latest commit.
    ///
    /// The default opts the object out of the summary and of acknowledgment
    /// notifications. Implementations opt in only when they own an actual
    /// configured or implied event algorithm and shared committed-transition
    /// history. Not public API yet (#2179).
    #[doc(hidden)]
    fn enrollment_summary_capability(&self) -> Option<EnrollmentSummaryCapability> {
        None
    }

    /// The counts the last committed BUFFER_READY transition reported, for
    /// the server to carry as its event values (Clause 13.3.7). `None` for an
    /// object that hasn't reported one, or doesn't run that algorithm, the
    /// default. Not public API yet (#2179).
    #[doc(hidden)]
    fn buffer_ready_report(&self) -> Option<BufferReadyReport> {
        None
    }

    /// The local property this object's Event_Algorithm_Inhibit follows, its
    /// Event_Algorithm_Inhibit_Ref (#1329); `None` while it holds none, and
    /// for an object without one, the default.
    ///
    /// The database reads the property and hands the value over through
    /// [`follow_event_algorithm_inhibit`](Self::follow_event_algorithm_inhibit);
    /// see [`ObjectDatabase::follow_event_algorithm_inhibit`](crate::database::ObjectDatabase::follow_event_algorithm_inhibit).
    fn event_algorithm_inhibit_reference(&self) -> Option<BACnetObjectPropertyReference> {
        None
    }

    /// Take the value the database read from the property
    /// [`event_algorithm_inhibit_reference`](Self::event_algorithm_inhibit_reference)
    /// names as Event_Algorithm_Inhibit; returns whether it changed. The
    /// default follows nothing.
    fn follow_event_algorithm_inhibit(&mut self, _inhibit: bool) -> bool {
        false
    }
}

/// The two accessors of an object that lends itself as its
/// [`EventReporting`], for inside its `BACnetObject` impl.
macro_rules! lend_event_reporting {
    () => {
        #[cfg(feature = "event-reporting")]
        fn event_reporting(&self) -> Option<&dyn $crate::traits::EventReporting> {
            Some(self)
        }

        #[cfg(feature = "event-reporting")]
        fn event_reporting_mut(&mut self) -> Option<&mut dyn $crate::traits::EventReporting> {
            Some(self)
        }
    };
}

pub(crate) use lend_event_reporting;
