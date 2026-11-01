//! The probe's Event Enrollment evaluation and reliability evaluation,
//! logged under `EnrollmentEval::` and `ReliabilityEval::`: every method
//! answers unlike its default.

use super::{Error, EventTransitionCommitError, P, Probe, Reliability};
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentReliabilityCommit,
};
use bacnet_objects::traits::{EnrollmentEval, ReliabilityEval, ReliabilityEvaluation};

impl EnrollmentEval for Probe {
    fn eval_state(&self) -> EventEnrollmentEvalState {
        self.called("EnrollmentEval::eval_state", ());
        EventEnrollmentEvalState {
            last_offnormal_value: Some(9),
            ..EventEnrollmentEvalState::default()
        }
    }
    fn set_eval_state(&mut self, state: EventEnrollmentEvalState) -> Result<(), Error> {
        self.called("EnrollmentEval::set_eval_state", (state,));
        Ok(())
    }
    fn eval_source(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        self.called("EnrollmentEval::eval_source", ());
        Some(Some((self.oid, P::PRESENT_VALUE, Some(2))))
    }
    fn set_eval_source(
        &mut self,
        source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        self.called("EnrollmentEval::set_eval_source", (source,));
        Ok(())
    }
    fn commit_reliability(
        &mut self,
        commit: EventEnrollmentReliabilityCommit,
    ) -> Result<(), EventTransitionCommitError> {
        self.called("EnrollmentEval::commit_reliability", (commit,));
        Ok(())
    }
}

impl ReliabilityEval for Probe {
    fn evaluate_reliability(&mut self) -> Result<ReliabilityEvaluation, Error> {
        self.called("ReliabilityEval::evaluate_reliability", ());
        Ok(ReliabilityEvaluation::Changed {
            old_reliability: Reliability::NO_SENSOR,
            new_reliability: Reliability::OVER_RANGE,
        })
    }
    fn reliability_evaluation_inhibited(&self) -> bool {
        self.called("ReliabilityEval::reliability_evaluation_inhibited", ());
        true
    }
    fn set_reliability(&mut self, reliability: Reliability) -> Result<(), Error> {
        self.called("ReliabilityEval::set_reliability", (reliability,));
        Ok(())
    }
}
