//! One Event Enrollment's evaluation in a pass, phase by phase: what the
//! pass reads from the enrollment, its local configuration, the monitored
//! object and the fault algorithm, up to the monitored value, which it
//! hands to [`detect`](super::detect) for the event algorithm. A phase that
//! settles the enrollment's outcome for this pass stages it in `updates` and
//! returns `None`, and the evaluation ends there.

use alloc::collections::{BTreeMap, BTreeSet};
use bacnet_objects::database::{LocalDevice, ObjectDatabase};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_objects::property_metadata::is_array_property;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{EventState, EventType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::commit::EnrollmentUpdate;
use super::detect::{count_down_and_fire, select_algorithm};
use super::fault::{
    FaultAlgorithmEvaluation, MonitoredReliability, SupportedFaultAlgorithm,
    evaluate_fault_algorithm, read_event_parameters, read_fault_algorithm,
    read_monitored_reliability,
};
use super::reference::{MonitoredReference, read_object_property_ref};
use super::support::*;
use super::{EventEnrollmentReliabilityCause, LocalConfigurationReadError};
use crate::event::CapturedReferencedValue;

/// What the pass read from one enrollment before evaluating it, which every
/// phase reads and none changes.
pub(super) struct EnrollmentInputs<'a> {
    pub(super) db: &'a ObjectDatabase,
    pub(super) oid: &'a ObjectIdentifier,
    pub(super) enrollment: &'a dyn BACnetObject,
    pub(super) local_device: LocalDevice,
    pub(super) interval_secs: u32,
    pub(super) eval_state_supported: bool,
    pub(super) eval_source: Option<Option<EventEnrollmentMonitoredSource>>,
    pub(super) event_type: EventType,
    pub(super) current_state: EventState,
    pub(super) event_enable: EventTransitionBits,
    pub(super) current_reliability: Reliability,
    pub(super) subject: ReliabilitySubject<'a>,
}

/// The monitored property the configuration names, and whether this pass
/// starts its private evaluation state over.
#[derive(Clone, Copy)]
pub(super) struct MonitoredTarget {
    pub(super) monitored: MonitoredReference,
    pub(super) monitored_oid: ObjectIdentifier,
    pub(super) monitored_prop: PropertyIdentifier,
    pub(super) monitored_reference: EventEnrollmentMonitoredSource,
    pub(super) reset_evaluation_state: bool,
}

/// Evaluate the Event Enrollment `oid` for this pass and stage what it
/// changes in `updates`.
pub(super) fn evaluate_enrollment(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    local_device: LocalDevice,
    interval_secs: u32,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    database_eval_sources: &mut BTreeSet<ObjectIdentifier>,
) {
    let Some(enrollment) = db.get(oid) else {
        return;
    };

    // Configuration-read exits are classified below. A valid monitored
    // or supporting observation gap applies repository-local D4: private
    // continuity is reset before its typed diagnostic is returned.
    let eval = enrollment.enrollment_eval();
    let eval_state_supported = eval.is_some();
    let mut eval_state = eval.map(|eval| eval.eval_state()).unwrap_or_default();
    let eval_source = match eval.and_then(|eval| eval.eval_source()) {
        Some(source) => Some(source),
        None if eval_state_supported => {
            database_eval_sources.insert(*oid);
            Some(db.enrollment_eval_source(oid))
        }
        None => None,
    };
    let force_state_reset = db.enrollment_eval_state_invalidated(oid);

    // Clause 13.2.2.1 suspends evaluation and prohibits transitions while
    // Event_Detection_Enable is FALSE. The accompanying reset is applied by the
    // object when the property is written (Clause 12.12 states the disabled
    // condition as an invariant), so skipping here cannot strand a stale
    // non-NORMAL state the way the pre-#136 Event_Enable gate did, nor a
    // stale countdown — the object-side reset clears the pending state.
    //
    // An object that does not model the property at all reads as an error
    // here and is treated as enabled. The property is required (R) on both
    // Event Enrollment (Table 12-14) and Alert Enrollment (Table 12-61) and
    // optional on most other types, so absence is common and must not
    // silently disable detection.
    if let Ok(PropertyValue::Boolean(false)) =
        enrollment.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
    {
        return;
    }

    let event_type = match enrollment.read_property(PropertyIdentifier::EVENT_TYPE, None) {
        Ok(PropertyValue::Enumerated(v)) => EventType::from_raw(v),
        _ => return,
    };

    let current_state = match enrollment.read_property(PropertyIdentifier::EVENT_STATE, None) {
        Ok(PropertyValue::Enumerated(v)) => EventState::from_raw(v),
        _ => return,
    };

    let event_enable = match enrollment.read_property(PropertyIdentifier::EVENT_ENABLE, None) {
        Ok(PropertyValue::BitString { data, .. }) => EventTransitionBits::from_bacnet(&data),
        _ => EventTransitionBits::empty(),
    };

    let current_reliability = match enrollment.read_property(PropertyIdentifier::RELIABILITY, None)
    {
        Ok(PropertyValue::Enumerated(value)) => Reliability::from_raw(value),
        _ => {
            queue_observation_unavailable(updates, *oid);
            return;
        }
    };
    let subject = ReliabilitySubject {
        db,
        enrollment,
        enrollment_oid: *oid,
        previous: current_reliability,
        current_state,
        event_enable,
    };
    let inputs = EnrollmentInputs {
        db,
        oid,
        enrollment,
        local_device,
        interval_secs,
        eval_state_supported,
        eval_source,
        event_type,
        current_state,
        event_enable,
        current_reliability,
        subject,
    };

    let Some((monitored, params, fault_algorithm)) =
        read_configuration(&inputs, updates, &mut eval_state, force_state_reset)
    else {
        return;
    };

    let monitored_oid = monitored.object_identifier;
    let monitored_prop = monitored.property_identifier;
    let monitored_reference = (monitored_oid, monitored_prop, monitored.array_index);
    // Private evaluation state belongs to one exact monitored source. A
    // nonempty ownerless state cannot be adopted safely. Mutations remain
    // staged until phase 2; a later observation gap overwrites them with
    // the D4 ownerless default.
    let source_changed = match eval_source {
        Some(Some(current)) => current != monitored_reference,
        Some(None) => eval_state != EventEnrollmentEvalState::default(),
        None => false,
    };
    let reset_evaluation_state = source_changed || force_state_reset;
    if reset_evaluation_state {
        eval_state = EventEnrollmentEvalState::default();
    }
    let target = MonitoredTarget {
        monitored,
        monitored_oid,
        monitored_prop,
        monitored_reference,
        reset_evaluation_state,
    };

    let Some(monitored_obj) = check_monitored_object(&inputs, &target, updates) else {
        return;
    };
    let Some(monitored_value) = observe_value(
        &inputs,
        &target,
        updates,
        &mut eval_state,
        fault_algorithm,
        monitored_obj,
    ) else {
        return;
    };
    let Some(selected) = select_algorithm(
        &inputs,
        &target,
        updates,
        &mut eval_state,
        params,
        monitored_value,
    ) else {
        return;
    };
    count_down_and_fire(
        &inputs,
        &target,
        updates,
        eval_state,
        selected,
        monitored_obj,
    );
}

/// Read the enrollment's monitored reference, Event_Parameters and fault
/// algorithm. A malformed one stages CONFIGURATION_ERROR, an unavailable one
/// or a remote reference its own outcome, and either returns `None`.
fn read_configuration(
    inputs: &EnrollmentInputs<'_>,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    eval_state: &mut EventEnrollmentEvalState,
    force_state_reset: bool,
) -> Option<(
    MonitoredReference,
    BACnetEventParameter,
    SupportedFaultAlgorithm,
)> {
    let EnrollmentInputs {
        oid,
        enrollment,
        local_device,
        eval_state_supported,
        eval_source,
        subject,
        ..
    } = *inputs;

    // Local configuration is resolved before any target observation. A
    // malformed/missing required reference, malformed parameters, or an
    // unsupported configured fault alternative therefore wins the
    // Reliability precedence chain deterministically.
    let reference = read_object_property_ref(enrollment);
    let params = read_event_parameters(enrollment);
    let fault_algorithm = read_fault_algorithm(enrollment, local_device);
    if matches!(reference, Err(LocalConfigurationReadError::Malformed))
        || matches!(params, Err(LocalConfigurationReadError::Malformed))
        || matches!(fault_algorithm, Err(LocalConfigurationReadError::Malformed))
    {
        let monitored_oid = reference
            .as_ref()
            .ok()
            .map(|reference| reference.object_identifier);
        queue_eval_state_reset(
            updates,
            *oid,
            eval_state_supported,
            eval_state,
            force_state_reset,
        );
        if matches!(reference, Err(LocalConfigurationReadError::Malformed)) {
            queue_eval_source_reset(updates, *oid, eval_source);
        }
        queue_reliability_transition(
            updates,
            &subject,
            monitored_oid,
            Reliability::CONFIGURATION_ERROR,
            EventEnrollmentReliabilityCause::Configuration,
            CapturedReferencedValue::Unavailable,
        );
        return None;
    }

    if matches!(reference, Err(LocalConfigurationReadError::Unavailable)) {
        queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
        return None;
    }
    if matches!(params, Err(LocalConfigurationReadError::Unavailable)) {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    }
    if matches!(
        fault_algorithm,
        Err(LocalConfigurationReadError::Unavailable)
    ) {
        queue_observation_unavailable(updates, *oid);
        return None;
    }

    let monitored = reference.expect("validated reference");
    let params = params.expect("validated Event_Parameters");
    let fault_algorithm = fault_algorithm.expect("validated Fault_Parameters");

    // A well-formed remote target is temporarily unobservable in this
    // local-only slice. D4 clears only private continuity; no persistent
    // target-loss Reliability policy is inferred.
    if !local_device.is_local(monitored.device_identifier) {
        queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
        return None;
    }
    Some((monitored, params, fault_algorithm))
}

/// Find the monitored object and take its own Reliability: a missing object,
/// an index into a property that isn't an array, or a fault in the object
/// stages its outcome and returns `None`.
fn check_monitored_object<'a>(
    inputs: &EnrollmentInputs<'a>,
    target: &MonitoredTarget,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
) -> Option<&'a dyn BACnetObject> {
    let EnrollmentInputs {
        db,
        oid,
        eval_state_supported,
        eval_source,
        subject,
        ..
    } = *inputs;
    let MonitoredTarget {
        monitored,
        monitored_oid,
        monitored_prop,
        monitored_reference,
        reset_evaluation_state,
    } = *target;

    let Some(monitored_obj) = db.get(&monitored_oid) else {
        queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
        return None;
    };
    if monitored.array_index.is_some() && !is_array_property(monitored_obj, monitored_prop) {
        queue_invalid_reference(updates, *oid, eval_state_supported, eval_source);
        queue_reliability_transition(
            updates,
            &subject,
            Some(monitored_oid),
            Reliability::CONFIGURATION_ERROR,
            EventEnrollmentReliabilityCause::Configuration,
            CapturedReferencedValue::Unavailable,
        );
        return None;
    }

    match read_monitored_reliability(monitored_obj) {
        MonitoredReliability::ConfigurationError => {
            queue_evaluation_ownership(
                updates,
                *oid,
                eval_state_supported,
                reset_evaluation_state,
                eval_source,
                monitored_reference,
            );
            queue_reliability_transition(
                updates,
                &subject,
                Some(monitored_oid),
                Reliability::CONFIGURATION_ERROR,
                EventEnrollmentReliabilityCause::Configuration,
                CapturedReferencedValue::NotEvaluated,
            );
            return None;
        }
        MonitoredReliability::ObservationUnavailable => {
            queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
            return None;
        }
        MonitoredReliability::Value(reliability)
            if reliability != Reliability::NO_FAULT_DETECTED =>
        {
            queue_evaluation_ownership(
                updates,
                *oid,
                eval_state_supported,
                reset_evaluation_state,
                eval_source,
                monitored_reference,
            );
            queue_reliability_transition(
                updates,
                &subject,
                Some(monitored_oid),
                Reliability::MONITORED_OBJECT_FAULT,
                EventEnrollmentReliabilityCause::MonitoredObject,
                CapturedReferencedValue::NotEvaluated,
            );
            return None;
        }
        MonitoredReliability::Absent | MonitoredReliability::Value(_) => {}
    }
    Some(monitored_obj)
}

/// Run the fault algorithm, return a recovered enrollment to NORMAL, and
/// read the monitored value: a fault, a recovery or a value that can't be
/// read stages its outcome and returns `None`.
fn observe_value(
    inputs: &EnrollmentInputs<'_>,
    target: &MonitoredTarget,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    eval_state: &mut EventEnrollmentEvalState,
    fault_algorithm: SupportedFaultAlgorithm,
    monitored_obj: &dyn BACnetObject,
) -> Option<PropertyValue> {
    let EnrollmentInputs {
        db,
        oid,
        eval_state_supported,
        eval_source,
        current_state,
        current_reliability,
        subject,
        ..
    } = *inputs;
    let MonitoredTarget {
        monitored,
        monitored_oid,
        monitored_prop,
        monitored_reference,
        reset_evaluation_state,
    } = *target;

    let mut monitored_value = None;
    if matches!(fault_algorithm, SupportedFaultAlgorithm::OutOfRange { .. }) {
        match db.read_object_property(monitored_obj, monitored_prop, monitored.array_index) {
            Ok(value) => monitored_value = Some(value),
            Err(error)
                if monitored.array_index.is_some() && invalid_indexed_target_error(&error) =>
            {
                queue_invalid_reference(updates, *oid, eval_state_supported, eval_source);
                queue_reliability_transition(
                    updates,
                    &subject,
                    Some(monitored_oid),
                    Reliability::CONFIGURATION_ERROR,
                    EventEnrollmentReliabilityCause::Configuration,
                    CapturedReferencedValue::Unavailable,
                );
                return None;
            }
            Err(_) => {
                queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
                return None;
            }
        }
    }

    match evaluate_fault_algorithm(db, &fault_algorithm, monitored_value.as_ref()) {
        FaultAlgorithmEvaluation::ConfigurationError => {
            queue_evaluation_ownership(
                updates,
                *oid,
                eval_state_supported,
                reset_evaluation_state,
                eval_source,
                monitored_reference,
            );
            queue_reliability_transition(
                updates,
                &subject,
                Some(monitored_oid),
                Reliability::CONFIGURATION_ERROR,
                EventEnrollmentReliabilityCause::Configuration,
                CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
            );
            return None;
        }
        FaultAlgorithmEvaluation::ObservationUnavailable => {
            queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
            return None;
        }
        FaultAlgorithmEvaluation::Fault(reliability) => {
            queue_evaluation_ownership(
                updates,
                *oid,
                eval_state_supported,
                reset_evaluation_state,
                eval_source,
                monitored_reference,
            );
            queue_reliability_transition(
                updates,
                &subject,
                Some(monitored_oid),
                reliability,
                EventEnrollmentReliabilityCause::FaultAlgorithm,
                CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
            );
            return None;
        }
        FaultAlgorithmEvaluation::Healthy => {}
    }

    if current_reliability != Reliability::NO_FAULT_DETECTED || current_state == EventState::FAULT {
        *eval_state = EventEnrollmentEvalState::default();
        queue_evaluation_ownership(
            updates,
            *oid,
            eval_state_supported,
            true,
            eval_source,
            monitored_reference,
        );
        let cause = if current_reliability == Reliability::CONFIGURATION_ERROR {
            EventEnrollmentReliabilityCause::Configuration
        } else if current_reliability == Reliability::MONITORED_OBJECT_FAULT {
            EventEnrollmentReliabilityCause::MonitoredObject
        } else {
            EventEnrollmentReliabilityCause::FaultAlgorithm
        };
        queue_reliability_transition(
            updates,
            &subject,
            Some(monitored_oid),
            Reliability::NO_FAULT_DETECTED,
            cause,
            CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
        );
        return None;
    }

    let monitored_value = match monitored_value {
        Some(value) => value,
        None => {
            match db.read_object_property(monitored_obj, monitored_prop, monitored.array_index) {
                Ok(value) => value,
                Err(error)
                    if monitored.array_index.is_some() && invalid_indexed_target_error(&error) =>
                {
                    queue_invalid_reference(updates, *oid, eval_state_supported, eval_source);
                    queue_reliability_transition(
                        updates,
                        &subject,
                        Some(monitored_oid),
                        Reliability::CONFIGURATION_ERROR,
                        EventEnrollmentReliabilityCause::Configuration,
                        CapturedReferencedValue::Unavailable,
                    );
                    return None;
                }
                Err(_) => {
                    queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
                    return None;
                }
            }
        }
    };

    queue_evaluation_ownership(
        updates,
        *oid,
        eval_state_supported,
        reset_evaluation_state,
        eval_source,
        monitored_reference,
    );
    Some(monitored_value)
}
