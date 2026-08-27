use std::collections::{HashMap, HashSet};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use bacnet_objects::event_enrollment::EventEnrollmentReliabilityCommit;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_types::enums::{EventState, EventType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::EventEnrollmentTransition;
use crate::server::event_timestamp::{confirm_event_timestamp, stage_event_timestamp};

/// Stage at which an Event Enrollment evaluation result was committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEnrollmentEvaluationStage {
    /// The enrollment produced no mutation or transition proposal.
    Evaluation,
    /// Monitored-source ownership was being updated.
    EvaluationSource,
    /// Private countdown or baseline state was being updated.
    EvaluationState,
    /// The atomic Event_State/Acked_Transitions/Event_Time_Stamps hook ran.
    EventTransition,
}

/// Observable result of one Event Enrollment commit stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEnrollmentEvaluationOutcome {
    /// Evaluation completed without a transition.
    NoTransition,
    /// A pending transition was canceled and its private state was stored.
    CancellationCommitted,
    /// A required internal mutation was rejected.
    Rejected,
    /// A custom hook returned an error after the target Event_State landed.
    ///
    /// This violates the atomic hook contract. The evaluator suppresses the
    /// transition token, does not consume the staged clockless sequence
    /// number, and invalidates private evaluation state for a later reset.
    LandedAfterError,
}

impl EventEnrollmentEvaluationOutcome {
    /// Whether this outcome represents a commit failure.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Rejected | Self::LandedAfterError)
    }
}

/// One structured Event Enrollment evaluation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventEnrollmentEvaluationDiagnostic {
    /// Enrollment whose evaluation produced this diagnostic.
    pub enrollment_oid: ObjectIdentifier,
    /// Commit stage that produced the outcome.
    pub stage: EventEnrollmentEvaluationStage,
    /// Observable stage outcome.
    pub outcome: EventEnrollmentEvaluationOutcome,
}

/// Stage exposed by the additive detailed Event Enrollment evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEnrollmentDetailedEvaluationStage {
    /// The enrollment produced no mutation or transition proposal.
    Evaluation,
    /// Monitored-source ownership was being updated.
    EvaluationSource,
    /// Private countdown or baseline state was being updated.
    EvaluationState,
    /// Reliability observation or the combined Reliability transition hook ran.
    Reliability,
    /// The atomic Event_State/Acked_Transitions/Event_Time_Stamps hook ran.
    EventTransition,
}

/// Outcome exposed by the additive detailed Event Enrollment evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEnrollmentDetailedEvaluationOutcome {
    /// Evaluation completed without a transition.
    NoTransition,
    /// A pending transition was canceled and its private state was stored.
    CancellationCommitted,
    /// A required local or remote observation was temporarily unavailable.
    ///
    /// No public Reliability, Event_State, acknowledgement, event-history,
    /// timestamp, or sequence transition is committed. The repository-local
    /// D4 policy may clear private continuity state and monitored-source
    /// ownership.
    ObservationUnavailable,
    /// A required internal mutation was rejected.
    Rejected,
    /// A custom hook returned an error after Reliability or Event_State landed.
    ///
    /// This violates the atomic hook contract. The evaluator suppresses the
    /// result token, does not consume the staged clockless sequence number,
    /// and invalidates private evaluation state for a later reset.
    LandedAfterError,
}

impl EventEnrollmentDetailedEvaluationOutcome {
    /// Whether this outcome represents a commit failure.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Rejected | Self::LandedAfterError)
    }
}

/// One diagnostic from the additive detailed Event Enrollment evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventEnrollmentDetailedEvaluationDiagnostic {
    /// Enrollment whose evaluation produced this diagnostic.
    pub enrollment_oid: ObjectIdentifier,
    /// Evaluation or commit stage that produced the outcome.
    pub stage: EventEnrollmentDetailedEvaluationStage,
    /// Observable stage outcome.
    pub outcome: EventEnrollmentDetailedEvaluationOutcome,
}

/// Precedence source that selected a committed Event Enrollment Reliability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEnrollmentReliabilityCause {
    /// A definitive local configuration defect selected CONFIGURATION_ERROR.
    Configuration,
    /// The monitored object's nonzero Reliability selected MONITORED_OBJECT_FAULT.
    MonitoredObject,
    /// A supported configured fault algorithm selected the value or recovery.
    FaultAlgorithm,
}

/// One successfully committed Event Enrollment Reliability result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnrollmentReliabilityResult {
    /// Event Enrollment object whose atomic commit succeeded.
    pub enrollment_oid: ObjectIdentifier,
    /// Configured monitored object, absent for a malformed/missing reference.
    pub monitored_oid: Option<ObjectIdentifier>,
    /// Reliability value read before the commit.
    pub previous_reliability: Reliability,
    /// Reliability value stored by the commit.
    pub new_reliability: Reliability,
    /// FAULT entry, re-entry, or recovery committed with Reliability.
    pub state_change: Option<EventStateChange>,
    /// Whether Event_Enable permits distribution for the transition coordinate.
    pub distribute: bool,
    /// Precedence source that selected `new_reliability`.
    pub cause: EventEnrollmentReliabilityCause,
}

impl EventEnrollmentReliabilityResult {
    /// Select the event type for this committed Reliability result.
    ///
    /// `configured_algorithm` is the Event Enrollment object's configured
    /// Event_Type. `None` means that the result has no committed state change.
    pub fn event_type(&self, configured_algorithm: EventType) -> Option<EventType> {
        self.state_change
            .as_ref()
            .map(|change| change.event_type(configured_algorithm))
    }
}

/// Legacy result of one Event Enrollment evaluation pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEnrollmentEvaluationReport {
    /// Transitions whose complete atomic object commit succeeded.
    pub transitions: Vec<EventEnrollmentTransition>,
    /// Non-transition and failure diagnostics in enrollment evaluation order.
    pub diagnostics: Vec<EventEnrollmentEvaluationDiagnostic>,
}

/// Additive detailed result of one Event Enrollment evaluation pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEnrollmentDetailedEvaluationReport {
    /// Transitions whose complete atomic object commit succeeded.
    pub transitions: Vec<EventEnrollmentTransition>,
    /// Reliability results whose complete combined object commit succeeded.
    pub reliability_results: Vec<EventEnrollmentReliabilityResult>,
    /// Every detailed evaluation, observation, and commit diagnostic.
    pub diagnostics: Vec<EventEnrollmentDetailedEvaluationDiagnostic>,
}

impl EventEnrollmentDetailedEvaluationReport {
    pub(super) fn into_legacy(self) -> EventEnrollmentEvaluationReport {
        let diagnostics = self
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                let stage = match (diagnostic.stage, diagnostic.outcome) {
                    (
                        EventEnrollmentDetailedEvaluationStage::Reliability,
                        EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable
                        | EventEnrollmentDetailedEvaluationOutcome::NoTransition,
                    ) => EventEnrollmentEvaluationStage::Evaluation,
                    (EventEnrollmentDetailedEvaluationStage::Reliability, _) => {
                        EventEnrollmentEvaluationStage::EventTransition
                    }
                    (EventEnrollmentDetailedEvaluationStage::Evaluation, _) => {
                        EventEnrollmentEvaluationStage::Evaluation
                    }
                    (EventEnrollmentDetailedEvaluationStage::EvaluationSource, _) => {
                        EventEnrollmentEvaluationStage::EvaluationSource
                    }
                    (EventEnrollmentDetailedEvaluationStage::EvaluationState, _) => {
                        EventEnrollmentEvaluationStage::EvaluationState
                    }
                    (EventEnrollmentDetailedEvaluationStage::EventTransition, _) => {
                        EventEnrollmentEvaluationStage::EventTransition
                    }
                };
                let outcome = match diagnostic.outcome {
                    EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable => {
                        EventEnrollmentEvaluationOutcome::NoTransition
                    }
                    EventEnrollmentDetailedEvaluationOutcome::NoTransition => {
                        EventEnrollmentEvaluationOutcome::NoTransition
                    }
                    EventEnrollmentDetailedEvaluationOutcome::CancellationCommitted => {
                        EventEnrollmentEvaluationOutcome::CancellationCommitted
                    }
                    EventEnrollmentDetailedEvaluationOutcome::Rejected => {
                        EventEnrollmentEvaluationOutcome::Rejected
                    }
                    EventEnrollmentDetailedEvaluationOutcome::LandedAfterError => {
                        EventEnrollmentEvaluationOutcome::LandedAfterError
                    }
                };
                EventEnrollmentEvaluationDiagnostic {
                    enrollment_oid: diagnostic.enrollment_oid,
                    stage,
                    outcome,
                }
            })
            .collect();
        EventEnrollmentEvaluationReport {
            transitions: self.transitions,
            diagnostics,
        }
    }
}

pub(crate) fn log_evaluation_report(report: &EventEnrollmentDetailedEvaluationReport) {
    for result in &report.reliability_results {
        tracing::debug!(
            enrollment = %result.enrollment_oid,
            monitored = ?result.monitored_oid,
            previous = ?result.previous_reliability,
            new = ?result.new_reliability,
            state_change = ?result.state_change,
            distribute = result.distribute,
            cause = ?result.cause,
            "Event enrollment: reliability committed"
        );
    }
    for diagnostic in report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.outcome.is_failure())
    {
        tracing::warn!(
            enrollment = %diagnostic.enrollment_oid,
            stage = ?diagnostic.stage,
            outcome = ?diagnostic.outcome,
            "Event enrollment: internal commit failed"
        );
    }
}

/// Coalesced desired mutation for one enrollment and one evaluation pass.
#[derive(Default)]
pub(super) struct EnrollmentUpdate {
    eval_state: Option<EventEnrollmentEvalState>,
    eval_source: Option<Option<EventEnrollmentMonitoredSource>>,
    clears_invalidation: bool,
    fired: Option<FiredTransition>,
    reliability: Option<ReliabilityUpdate>,
    observation_unavailable: bool,
    canceled: bool,
}

pub(super) struct FiredTransition {
    pub(super) monitored_oid: ObjectIdentifier,
    pub(super) event_type_raw: u32,
    pub(super) from: EventState,
    pub(super) to: EventState,
    pub(super) distribute: bool,
    pub(super) ack_required: bool,
}

pub(super) struct ReliabilityUpdate {
    pub(super) monitored_oid: Option<ObjectIdentifier>,
    pub(super) previous: Reliability,
    pub(super) desired: Reliability,
    pub(super) from: EventState,
    pub(super) to: EventState,
    pub(super) distribute: bool,
    pub(super) ack_required: bool,
    pub(super) cause: EventEnrollmentReliabilityCause,
}

impl EnrollmentUpdate {
    /// Replace the desired private state with the state reached later in the
    /// same deterministic phase-1 flow. This is semantic coalescing: reset,
    /// cancellation, and reseed operations all update one final snapshot.
    pub(super) fn set_eval_state(&mut self, state: EventEnrollmentEvalState) {
        self.eval_state = Some(state);
    }

    pub(super) fn reset_eval_state(&mut self) {
        self.eval_state = Some(EventEnrollmentEvalState::default());
        self.clears_invalidation = true;
    }

    pub(super) fn cancel_pending(&mut self, state: EventEnrollmentEvalState) {
        self.eval_state = Some(state);
        self.canceled = true;
    }

    pub(super) fn set_eval_source(&mut self, source: Option<EventEnrollmentMonitoredSource>) {
        self.eval_source = Some(source);
    }

    /// Replace every source mutation staged earlier in this pass with the
    /// minimal owner-clear required by the pre-pass source coordinate.
    pub(super) fn reset_eval_source_for_observation_gap(
        &mut self,
        previous: Option<Option<EventEnrollmentMonitoredSource>>,
    ) {
        self.eval_source = match previous {
            Some(Some(_)) => Some(None),
            Some(None) | None => None,
        };
    }

    pub(super) fn fire(&mut self, fired: FiredTransition) {
        debug_assert!(
            self.fired.is_none(),
            "one enrollment fires at most once per pass"
        );
        self.fired = Some(fired);
    }

    pub(super) fn set_reliability(&mut self, reliability: ReliabilityUpdate) {
        debug_assert!(
            self.reliability.is_none() && self.fired.is_none(),
            "one enrollment commits one transition family per pass"
        );
        self.reliability = Some(reliability);
    }

    pub(super) fn observation_unavailable(&mut self) {
        // A terminal gap cannot leak a transition staged earlier in the same
        // deterministic phase-1 flow.
        self.fired = None;
        self.reliability = None;
        self.canceled = false;
        self.observation_unavailable = true;
    }
}

fn diagnostic(
    enrollment_oid: ObjectIdentifier,
    stage: EventEnrollmentDetailedEvaluationStage,
    outcome: EventEnrollmentDetailedEvaluationOutcome,
) -> EventEnrollmentDetailedEvaluationDiagnostic {
    EventEnrollmentDetailedEvaluationDiagnostic {
        enrollment_oid,
        stage,
        outcome,
    }
}

fn event_state_landed(db: &ObjectDatabase, oid: &ObjectIdentifier, state: EventState) -> bool {
    matches!(
        db.get(oid)
            .and_then(|object| object.read_property(PropertyIdentifier::EVENT_STATE, None).ok()),
        Some(PropertyValue::Enumerated(raw)) if raw == state.to_raw()
    )
}

fn reliability_or_state_landed(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    update: &ReliabilityUpdate,
) -> bool {
    let Some(object) = db.get(oid) else {
        return false;
    };
    let reliability_landed = update.previous != update.desired
        && matches!(
            object.read_property(PropertyIdentifier::RELIABILITY, None),
            Ok(PropertyValue::Enumerated(raw)) if raw == update.desired.to_raw()
        );
    let state_landed = update.from != update.to
        && matches!(
            object.read_property(PropertyIdentifier::EVENT_STATE, None),
            Ok(PropertyValue::Enumerated(raw)) if raw == update.to.to_raw()
        );
    reliability_landed || state_landed
}

fn clear_source_ownership(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    database_eval_sources: &HashSet<ObjectIdentifier>,
) {
    if database_eval_sources.contains(&oid) {
        db.set_enrollment_eval_source(oid, None);
    } else if let Some(object) = db.get_mut(&oid) {
        let _ = object.set_enrollment_eval_source_internal(None);
    }
}

pub(super) fn apply_updates(
    db: &mut ObjectDatabase,
    oids: &[ObjectIdentifier],
    mut updates: HashMap<ObjectIdentifier, EnrollmentUpdate>,
    database_eval_sources: &HashSet<ObjectIdentifier>,
) -> EventEnrollmentDetailedEvaluationReport {
    let mut report = EventEnrollmentDetailedEvaluationReport::default();

    for &oid in oids {
        let Some(update) = updates.remove(&oid) else {
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentDetailedEvaluationStage::Evaluation,
                EventEnrollmentDetailedEvaluationOutcome::NoTransition,
            ));
            continue;
        };

        let mut source_failed = false;
        let mut state_failed = false;
        if let Some(source) = update.eval_source {
            if database_eval_sources.contains(&oid) {
                db.set_enrollment_eval_source(oid, source);
            } else {
                let failed = db.get_mut(&oid).is_none_or(|object| {
                    object.set_enrollment_eval_source_internal(source).is_err()
                });
                if failed {
                    source_failed = true;
                    report.diagnostics.push(diagnostic(
                        oid,
                        EventEnrollmentDetailedEvaluationStage::EvaluationSource,
                        EventEnrollmentDetailedEvaluationOutcome::Rejected,
                    ));
                }
            }
        }

        let mut state_committed = false;
        if source_failed {
            // Dependent state cannot retain an owner that failed to land.
            // This is the single allowed private-state setter call this pass.
            state_failed = db.get_mut(&oid).is_none_or(|object| {
                object
                    .set_enrollment_eval_state_internal(EventEnrollmentEvalState::default())
                    .is_err()
            });
            if state_failed {
                db.set_enrollment_eval_state_invalidated(oid, true);
                report.diagnostics.push(diagnostic(
                    oid,
                    EventEnrollmentDetailedEvaluationStage::EvaluationState,
                    EventEnrollmentDetailedEvaluationOutcome::Rejected,
                ));
            } else {
                state_committed = true;
            }
        } else if let Some(state) = update.eval_state {
            state_failed = db
                .get_mut(&oid)
                .is_none_or(|object| object.set_enrollment_eval_state_internal(state).is_err());
            if state_failed {
                clear_source_ownership(db, oid, database_eval_sources);
                db.set_enrollment_eval_state_invalidated(oid, true);
                report.diagnostics.push(diagnostic(
                    oid,
                    EventEnrollmentDetailedEvaluationStage::EvaluationState,
                    EventEnrollmentDetailedEvaluationOutcome::Rejected,
                ));
            } else {
                state_committed = true;
                if update.clears_invalidation {
                    db.set_enrollment_eval_state_invalidated(oid, false);
                }
            }
        }

        if update.observation_unavailable {
            if source_failed || state_failed {
                db.set_enrollment_eval_state_invalidated(oid, true);
            }
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentDetailedEvaluationStage::Reliability,
                EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable,
            ));
            continue;
        }

        if state_failed {
            continue;
        }

        if let Some(reliability) = update.reliability {
            let coordinate = EventTransition::for_target_state(reliability.to);
            let staged_timestamp = stage_event_timestamp(db);
            let change = EventStateChange {
                from: reliability.from,
                to: reliability.to,
            };
            let transition = EventTransitionCommit {
                change: change.clone(),
                coordinate,
                ack_required: reliability.ack_required,
                timestamp: staged_timestamp.sample.timestamp.clone(),
                message_text: None,
            };
            let commit = EventEnrollmentReliabilityCommit {
                reliability: reliability.desired,
                transition: Some(transition),
            };
            let commit_result = db
                .get_mut(&oid)
                .map_or(Err(EventTransitionCommitError::Unsupported), |object| {
                    object.commit_event_enrollment_reliability_internal(commit)
                });

            if commit_result.is_err() {
                let outcome = if reliability_or_state_landed(db, &oid, &reliability) {
                    EventEnrollmentDetailedEvaluationOutcome::LandedAfterError
                } else {
                    EventEnrollmentDetailedEvaluationOutcome::Rejected
                };
                clear_source_ownership(db, oid, database_eval_sources);
                db.set_enrollment_eval_state_invalidated(oid, true);
                report.diagnostics.push(diagnostic(
                    oid,
                    EventEnrollmentDetailedEvaluationStage::Reliability,
                    outcome,
                ));
                continue;
            }

            confirm_event_timestamp(db, staged_timestamp);
            report
                .reliability_results
                .push(EventEnrollmentReliabilityResult {
                    enrollment_oid: oid,
                    monitored_oid: reliability.monitored_oid,
                    previous_reliability: reliability.previous,
                    new_reliability: reliability.desired,
                    state_change: Some(change),
                    distribute: reliability.distribute,
                    cause: reliability.cause,
                });
            continue;
        }

        let Some(fired) = update.fired else {
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentDetailedEvaluationStage::EvaluationState,
                if update.canceled && state_committed {
                    EventEnrollmentDetailedEvaluationOutcome::CancellationCommitted
                } else {
                    EventEnrollmentDetailedEvaluationOutcome::NoTransition
                },
            ));
            continue;
        };

        // A same-state normal-event transition depends on private source
        // ownership to distinguish the new indication from the previous one.
        // Reliability re-entry is independently distinguished by its changed
        // Reliability value and therefore is not suppressed by source failure.
        if source_failed && fired.from == fired.to {
            continue;
        }

        let coordinate = EventTransition::for_target_state(fired.to);
        let staged_timestamp = stage_event_timestamp(db);
        let change = EventStateChange {
            from: fired.from,
            to: fired.to,
        };
        let commit = EventTransitionCommit {
            change: change.clone(),
            coordinate,
            ack_required: fired.ack_required,
            timestamp: staged_timestamp.sample.timestamp.clone(),
            message_text: None,
        };
        let commit_result = db
            .get_mut(&oid)
            .map_or(Err(EventTransitionCommitError::Unsupported), |object| {
                object.commit_event_transition_internal(commit)
            });

        if commit_result.is_err() {
            let outcome = if fired.from != fired.to && event_state_landed(db, &oid, fired.to) {
                EventEnrollmentDetailedEvaluationOutcome::LandedAfterError
            } else {
                EventEnrollmentDetailedEvaluationOutcome::Rejected
            };
            clear_source_ownership(db, oid, database_eval_sources);
            db.set_enrollment_eval_state_invalidated(oid, true);
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentDetailedEvaluationStage::EventTransition,
                outcome,
            ));
            continue;
        }

        confirm_event_timestamp(db, staged_timestamp);
        let event_type = change.event_type(EventType::from_raw(fired.event_type_raw));
        report.transitions.push(EventEnrollmentTransition {
            enrollment_oid: oid,
            monitored_oid: fired.monitored_oid,
            change,
            event_type,
            distribute: fired.distribute,
        });
    }

    report
}
