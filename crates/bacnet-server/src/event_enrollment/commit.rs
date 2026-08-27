use std::collections::{HashMap, HashSet};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_types::enums::{EventState, EventType, PropertyIdentifier};
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

/// Detailed result of one Event Enrollment evaluation pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEnrollmentEvaluationReport {
    /// Transitions whose complete atomic object commit succeeded.
    pub transitions: Vec<EventEnrollmentTransition>,
    /// Non-transition and failure diagnostics in enrollment evaluation order.
    pub diagnostics: Vec<EventEnrollmentEvaluationDiagnostic>,
}

pub(crate) fn log_evaluation_failures(report: &EventEnrollmentEvaluationReport) {
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

    pub(super) fn fire(&mut self, fired: FiredTransition) {
        debug_assert!(
            self.fired.is_none(),
            "one enrollment fires at most once per pass"
        );
        self.fired = Some(fired);
    }
}

fn diagnostic(
    enrollment_oid: ObjectIdentifier,
    stage: EventEnrollmentEvaluationStage,
    outcome: EventEnrollmentEvaluationOutcome,
) -> EventEnrollmentEvaluationDiagnostic {
    EventEnrollmentEvaluationDiagnostic {
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
) -> EventEnrollmentEvaluationReport {
    let mut report = EventEnrollmentEvaluationReport::default();

    for &oid in oids {
        let Some(update) = updates.remove(&oid) else {
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentEvaluationStage::Evaluation,
                EventEnrollmentEvaluationOutcome::NoTransition,
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
                        EventEnrollmentEvaluationStage::EvaluationSource,
                        EventEnrollmentEvaluationOutcome::Rejected,
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
                    EventEnrollmentEvaluationStage::EvaluationState,
                    EventEnrollmentEvaluationOutcome::Rejected,
                ));
            } else {
                state_committed = true;
            }
        } else if let Some(state) = update.eval_state {
            let state_failed = db
                .get_mut(&oid)
                .is_none_or(|object| object.set_enrollment_eval_state_internal(state).is_err());
            if state_failed {
                clear_source_ownership(db, oid, database_eval_sources);
                db.set_enrollment_eval_state_invalidated(oid, true);
                report.diagnostics.push(diagnostic(
                    oid,
                    EventEnrollmentEvaluationStage::EvaluationState,
                    EventEnrollmentEvaluationOutcome::Rejected,
                ));
                continue;
            }
            state_committed = true;
            if update.clears_invalidation {
                db.set_enrollment_eval_state_invalidated(oid, false);
            }
        }

        let Some(fired) = update.fired else {
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentEvaluationStage::EvaluationState,
                if update.canceled && state_committed {
                    EventEnrollmentEvaluationOutcome::CancellationCommitted
                } else {
                    EventEnrollmentEvaluationOutcome::NoTransition
                },
            ));
            continue;
        };

        if state_failed {
            continue;
        }

        // A same-state transition depends on private state to distinguish the
        // new indication from the previous one. Preserve the established
        // stateless source-failure behavior only for a changing Event_State.
        if source_failed && fired.from == fired.to {
            continue;
        }

        let coordinate = EventTransition::for_target_state(fired.to);
        let staged_timestamp = stage_event_timestamp(db);
        let commit = EventTransitionCommit {
            change: EventStateChange {
                from: fired.from,
                to: fired.to,
            },
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
                EventEnrollmentEvaluationOutcome::LandedAfterError
            } else {
                EventEnrollmentEvaluationOutcome::Rejected
            };
            clear_source_ownership(db, oid, database_eval_sources);
            db.set_enrollment_eval_state_invalidated(oid, true);
            report.diagnostics.push(diagnostic(
                oid,
                EventEnrollmentEvaluationStage::EventTransition,
                outcome,
            ));
            continue;
        }

        confirm_event_timestamp(db, staged_timestamp);
        report.transitions.push(EventEnrollmentTransition {
            enrollment_oid: oid,
            monitored_oid: fired.monitored_oid,
            change: EventStateChange {
                from: fired.from,
                to: fired.to,
            },
            event_type: EventType::from_raw(fired.event_type_raw),
            distribute: fired.distribute,
        });
    }

    report
}
