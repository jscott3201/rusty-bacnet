//! The Schedule's own Reliability evaluation (#1056, #1086, #1433, #1436).
//!
//! Clause 12.24.13 ties Reliability to the schedule's configuration being
//! consistent, in two halves. When either fails, Reliability is
//! CONFIGURATION_ERROR, and Status_Flags reports FAULT through the usual
//! Reliability mapping.
//!
//! The contents: leaving NULLs aside, every value in Weekly_Schedule,
//! Exception_Schedule and Schedule_Default must be of a single datatype. The
//! object checks this on every change to them.
//!
//! The references: every member of List_Of_Object_Property_References must
//! accept that datatype. Only the target objects can say, so the object
//! learns it from its writes (#1086): the server reports how each target took
//! a write through `complete_schedule_write`, and a member that refused the
//! value for its datatype (INVALID_DATA_TYPE or DATATYPE_NOT_SUPPORTED) faults
//! the Schedule until a later write to it succeeds or it leaves the list. So
//! does a member the target can't write at all, whatever the value (#1433):
//! one naming a missing object or property, or an array index the property
//! can't take (UNKNOWN_OBJECT, UNKNOWN_PROPERTY, PROPERTY_IS_NOT_AN_ARRAY,
//! INVALID_ARRAY_INDEX). Such a member clears once a later write to it
//! succeeds, say after the object is created, or once it leaves the list.
//!
//! The fault therefore shows at the first write, not at configuration time; the
//! clause already lets a remote member's fault wait for a write. Only a value
//! of a datatype the schedule itself holds counts, so a NULL, or a value of
//! another datatype a client wrote to Present_Value out of service, says
//! nothing about the configuration. Any other failure raises nothing, and on a
//! write that isn't a retry (below) it leaves a member's standing as it was: a
//! denied write, for one, can come from the target's state and not its
//! configuration. Deciding beforehand instead would need a model of each target
//! property's datatype, which the objects don't publish, or a guess from the
//! target's current value.
//!
//! A refusal can outlive its cause: the missing object is created later, or the
//! array grows to take the index. The Schedule writes its list only when its
//! value changes, when it enters Effective_Period and when its references or
//! priority change, so a Schedule whose value stays put would never ask again
//! (#1436). So a pass that owes the list nothing still offers the current value
//! to the refused members, and to them only: the members that took it, a
//! Command or Channel among them, are not written again, and the slots the
//! Schedule holds stay as they were. A member that takes the retry clears as
//! after any accepted write. One that fails it otherwise, say with a value its
//! new object finds out of range, clears too: the target no longer refuses it,
//! so it stands as it would had that been its first write, and the order in
//! which a configuration was built doesn't decide the fault. A NULL
//! Present_Value isn't retried, since a NULL never counts here, and nothing is
//! retried out of service or outside Effective_Period, where the calculation
//! sends nothing. The server doesn't wait for a pass when the object a
//! refused member names is added to the database (#1440): it offers the
//! value to the members naming that object at once
//! ([`retry_refusals_naming`](crate::traits::BACnetObject::retry_refusals_naming)).
//! An array grown to take the index still waits for the pass. Datatype refusals are retried as well as reference refusals:
//! both answer the one question of 12.24.13, and a datatype refusal can pass
//! without the Schedule changing too, when the application puts an object that
//! takes the datatype under the target's identifier. A member that still
//! refuses costs one refused write per pass and changes nothing.
//!
//! A misconfigured schedule keeps evaluating and writing its references.
//! Clause 12.24.4 makes those writes unconditional, Reliability only reports
//! the inconsistency, and a target that cannot take a value refuses that one
//! write without stopping the others. The multi-state objects treat their
//! CONFIGURATION_ERROR the same way.
//!
//! The evaluation owns only the fault it raised. A Reliability applied through
//! `set_reliability_internal` stays until that caller changes it, and while
//! Out_Of_Service is TRUE the client's simulated value is left alone; the
//! return to service restores the saved value and evaluates again. Refusals
//! reported meanwhile are kept for that evaluation.

use core::mem::discriminant;

use bacnet_types::enums::Reliability;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{ScheduleObject, ScheduleTargetOutcome, ScheduleWrite};
use crate::traits::ReliabilityEvaluation;

impl ScheduleObject {
    /// The non-NULL values of Weekly_Schedule, Exception_Schedule and
    /// Schedule_Default.
    fn scheduled_values(&self) -> impl Iterator<Item = &PropertyValue> {
        let weekly = self.weekly_schedule.iter().flatten();
        let exceptions = self
            .exception_schedule
            .iter()
            .flat_map(|event| &event.list_of_time_values);
        weekly
            .chain(exceptions)
            .map(|time_value| &time_value.value)
            .chain(core::iter::once(&self.schedule_default))
            .filter(|value| **value != PropertyValue::Null)
    }

    /// Whether the non-NULL values of Weekly_Schedule, Exception_Schedule and
    /// Schedule_Default are all of one datatype. With no such values at all
    /// there is nothing to disagree.
    pub(super) fn values_share_one_datatype(&self) -> bool {
        let mut datatypes = self.scheduled_values().map(discriminant);
        match datatypes.next() {
            Some(first) => datatypes.all(|datatype| datatype == first),
            None => true,
        }
    }

    /// Take how each target took `write`, one outcome per reference in order,
    /// and re-check Reliability; returns whether it changed.
    pub(super) fn complete_write(
        &mut self,
        write: &ScheduleWrite,
        outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        let datatype = discriminant(&write.value);
        if !self
            .scheduled_values()
            .any(|value| discriminant(value) == datatype)
        {
            return false;
        }
        for (reference, outcome) in write.references.iter().zip(outcomes) {
            match outcome {
                ScheduleTargetOutcome::Accepted => {
                    self.refusing_references
                        .retain(|refusing| refusing != reference);
                }
                // A retry that fails otherwise isn't refused either: the
                // member stands as if that had been its first write.
                ScheduleTargetOutcome::Failed if write.retry => {
                    self.refusing_references
                        .retain(|refusing| refusing != reference);
                }
                ScheduleTargetOutcome::DatatypeRefused
                | ScheduleTargetOutcome::ReferenceRefused
                    if self.list_of_object_property_references.contains(reference)
                        && !self.refusing_references.contains(reference) =>
                {
                    self.refusing_references.push(reference.clone());
                }
                _ => {}
            }
        }
        matches!(
            self.recompute_reliability(),
            ReliabilityEvaluation::Changed { .. }
        )
    }

    /// The write that offers Present_Value again to the references whose
    /// last write was refused (#1436), at Priority_For_Writing; `None` with
    /// no refusal standing or a NULL Present_Value, which can't clear one.
    pub(super) fn retry_refused(&self) -> Option<ScheduleWrite> {
        if self.refusing_references.is_empty() || self.present_value == PropertyValue::Null {
            return None;
        }
        Some(ScheduleWrite {
            value: self.present_value.clone(),
            priority: self.priority_for_writing,
            references: self.refusing_references.clone(),
            retry: true,
        })
    }

    /// [`retry_refused`](Self::retry_refused) limited to the references
    /// naming `target`, for an object just created under that identifier
    /// (#1440). Only while the calculation would send a retry itself: in
    /// service and in Effective_Period as the last pass found it.
    pub(super) fn retry_refused_naming(&self, target: ObjectIdentifier) -> Option<ScheduleWrite> {
        if self.out_of_service || !self.in_effective_period {
            return None;
        }
        if !self
            .refusing_references
            .iter()
            .any(|reference| reference.object_identifier == target)
        {
            return None;
        }
        let mut write = self.retry_refused()?;
        write
            .references
            .retain(|reference| reference.object_identifier == target);
        Some(write)
    }

    /// Re-run the consistency check after the contents or the references
    /// changed, a write was reported, or the object returned to service.
    ///
    /// Raises CONFIGURATION_ERROR over NO_FAULT_DETECTED, and clears it only
    /// if this evaluation raised it. Does nothing while Out_Of_Service is
    /// TRUE.
    pub(super) fn recompute_reliability(&mut self) -> ReliabilityEvaluation {
        if self.out_of_service {
            return ReliabilityEvaluation::Unchanged;
        }
        let misconfigured =
            !self.values_share_one_datatype() || !self.refusing_references.is_empty();
        let new_reliability = if self.owns_configuration_error {
            if misconfigured {
                Reliability::CONFIGURATION_ERROR
            } else {
                Reliability::NO_FAULT_DETECTED
            }
        } else if misconfigured && self.reliability == Reliability::NO_FAULT_DETECTED {
            Reliability::CONFIGURATION_ERROR
        } else {
            return ReliabilityEvaluation::Unchanged;
        };
        self.owns_configuration_error = misconfigured;
        if new_reliability == self.reliability {
            return ReliabilityEvaluation::Unchanged;
        }
        let old_reliability = core::mem::replace(&mut self.reliability, new_reliability);
        ReliabilityEvaluation::Changed {
            old_reliability,
            new_reliability,
        }
    }
}
