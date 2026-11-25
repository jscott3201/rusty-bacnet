//! The Event Enrollment evaluation pass and its commit. The module docs of
//! `event::enrollment` describe what it runs and how.

mod commit;
mod detect;
mod evaluate;
mod support;

use alloc::collections::{BTreeMap, BTreeSet};

pub use commit::{
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult,
};

#[cfg(test)]
use commit::apply_updates;
pub use commit::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult,
    EventEnrollmentEvaluationBatch,
};
use commit::{EnrollmentUpdate, apply_updates_for_delivery};
pub use commit::{EventEnrollmentTransition, SuppressedDelivery, SuppressionCause};
use evaluate::evaluate_enrollment;
#[cfg(test)]
use reference::MonitoredReference;
#[cfg(test)]
use reference::{params_fingerprint, read_object_property_ref};
#[cfg(test)]
use support::passes_for_delay;

use super::{LocalConfigurationReadError, algorithms, fault, reference};

use bacnet_objects::database::ObjectDatabase;
#[cfg(test)]
use bacnet_objects::event::{EventStateChange, EventTransition};
#[cfg(test)]
use bacnet_objects::event_enrollment::EventEnrollmentEvalState;
#[cfg(test)]
use bacnet_objects::traits::BACnetObject;
#[cfg(test)]
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::ObjectType;
#[cfg(test)]
use bacnet_types::enums::{EventState, EventType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
#[cfg(test)]
use bacnet_types::primitives::PropertyValue;

/// Run one evaluation pass over every Event Enrollment object in `db`,
/// commit what it finds, and return the report with the commits to send.
///
/// `interval_secs` is the host's time between passes, in whole seconds (0
/// counts as 1): a time delay becomes that many passes, rounded up, so it
/// never fires early. A host whose period isn't whole seconds rounds it
/// down, or delays fire early. It is a `u32` so the conversion is a 32-bit
/// division, which a 32-bit target does without a library routine. The
/// batch's `deliveries` include transitions whose `Event_Enable` bit is
/// clear; send only those whose `distribute()` is true.
pub fn evaluate_event_enrollments_for_delivery(
    db: &mut ObjectDatabase,
    interval_secs: u32,
) -> EventEnrollmentEvaluationBatch {
    let interval_secs = interval_secs.max(1);
    let oids = db.find_by_type(ObjectType::EVENT_ENROLLMENT);
    // Event_Algorithm_Inhibit follows its reference as of this pass too, on
    // top of the server's one-second tick (#1507).
    for oid in &oids {
        db.follow_event_algorithm_inhibit(oid);
    }
    // The monitored, setpoint and fault references all resolve through this
    // one rule: unqualified or naming this device is local (#1184).
    let local_device = db.local_device();

    let mut updates: BTreeMap<ObjectIdentifier, EnrollmentUpdate> = BTreeMap::new();
    let mut database_eval_sources = BTreeSet::new();

    for oid in &oids {
        evaluate_enrollment(
            db,
            oid,
            local_device,
            interval_secs,
            &mut updates,
            &mut database_eval_sources,
        );
    }

    apply_updates_for_delivery(db, &oids, updates, &database_eval_sources)
}

#[cfg(test)]
use tests::helpers::{evaluate_event_enrollments, evaluate_event_enrollments_report};

#[cfg(test)]
mod tests;
