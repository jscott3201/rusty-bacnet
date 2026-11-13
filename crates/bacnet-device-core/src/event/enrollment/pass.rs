//! The Event Enrollment evaluation pass and its commit. The module docs of
//! `event::enrollment` describe what it runs and how.

mod commit;
mod support;

use alloc::collections::{BTreeMap, BTreeSet};
use bacnet_objects::property_metadata::is_array_property;

pub use commit::{
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult,
};

use algorithms::{
    ArmEvaluation, eval_change_of_bitstring_struct, eval_change_of_state_struct,
    eval_change_of_value_struct, eval_floating_limit_struct, eval_legacy_le_arm, eval_none_arm,
    eval_out_of_range_struct, extract_bitstring, extract_property_state_value, extract_real,
    inhibited_arm,
};
#[cfg(test)]
use commit::apply_updates;
pub use commit::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult,
    EventEnrollmentEvaluationBatch,
};
use commit::{EnrollmentUpdate, FiredTransition, apply_updates_for_delivery};
pub use commit::{EventEnrollmentTransition, SuppressedDelivery};
use fault::{
    FaultAlgorithmEvaluation, MonitoredReliability, SupportedFaultAlgorithm,
    evaluate_fault_algorithm, read_event_parameters, read_fault_algorithm,
    read_monitored_reliability,
};
#[cfg(test)]
use reference::MonitoredReference;
use reference::{params_fingerprint, read_object_property_ref};
use support::*;

use super::{LocalConfigurationReadError, algorithms, fault, reference};

use crate::event::{
    CapturedReferencedValue, EventEnrollmentProjectionSnapshot, capture_status_flags,
};
use bacnet_objects::database::ObjectDatabase;
#[cfg(test)]
use bacnet_objects::event::EventStateChange;
use bacnet_objects::event::EventTransition;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentPending};
#[cfg(test)]
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// Run one evaluation pass over every Event Enrollment object in `db`,
/// commit what it finds, and return the report with the commits to send.
///
/// `interval_secs` is the host's time between passes, in seconds (0 counts
/// as 1): a time delay becomes that many passes, rounded up, so it never
/// fires early. The batch's `deliveries` include transitions whose
/// `Event_Enable` bit is clear; send only those whose result has
/// `distribute` set.
pub fn evaluate_event_enrollments_for_delivery(
    db: &mut ObjectDatabase,
    interval_secs: u64,
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
        let Some(enrollment) = db.get(oid) else {
            continue;
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
            continue;
        }

        let event_type = match enrollment.read_property(PropertyIdentifier::EVENT_TYPE, None) {
            Ok(PropertyValue::Enumerated(v)) => EventType::from_raw(v),
            _ => continue,
        };

        let current_state = match enrollment.read_property(PropertyIdentifier::EVENT_STATE, None) {
            Ok(PropertyValue::Enumerated(v)) => EventState::from_raw(v),
            _ => continue,
        };

        let event_enable = match enrollment.read_property(PropertyIdentifier::EVENT_ENABLE, None) {
            Ok(PropertyValue::BitString { data, .. }) => EventTransitionBits::from_bacnet(&data),
            _ => EventTransitionBits::empty(),
        };

        let current_reliability =
            match enrollment.read_property(PropertyIdentifier::RELIABILITY, None) {
                Ok(PropertyValue::Enumerated(value)) => Reliability::from_raw(value),
                _ => {
                    queue_observation_unavailable(&mut updates, *oid);
                    continue;
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
                &mut updates,
                *oid,
                eval_state_supported,
                &eval_state,
                force_state_reset,
            );
            if matches!(reference, Err(LocalConfigurationReadError::Malformed)) {
                queue_eval_source_reset(&mut updates, *oid, eval_source);
            }
            queue_reliability_transition(
                &mut updates,
                &subject,
                monitored_oid,
                Reliability::CONFIGURATION_ERROR,
                EventEnrollmentReliabilityCause::Configuration,
                CapturedReferencedValue::Unavailable,
            );
            continue;
        }

        if matches!(reference, Err(LocalConfigurationReadError::Unavailable)) {
            queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
            continue;
        }
        if matches!(params, Err(LocalConfigurationReadError::Unavailable)) {
            queue_pending_cancellation(&mut updates, *oid, eval_state_supported, &mut eval_state);
            continue;
        }
        if matches!(
            fault_algorithm,
            Err(LocalConfigurationReadError::Unavailable)
        ) {
            queue_observation_unavailable(&mut updates, *oid);
            continue;
        }

        let monitored = reference.expect("validated reference");
        let params = params.expect("validated Event_Parameters");
        let fault_algorithm = fault_algorithm.expect("validated Fault_Parameters");

        // A well-formed remote target is temporarily unobservable in this
        // local-only slice. D4 clears only private continuity; no persistent
        // target-loss Reliability policy is inferred.
        if !local_device.is_local(monitored.device_identifier) {
            queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
            continue;
        }

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

        let Some(monitored_obj) = db.get(&monitored_oid) else {
            queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
            continue;
        };
        if monitored.array_index.is_some() && !is_array_property(monitored_obj, monitored_prop) {
            queue_invalid_reference(&mut updates, *oid, eval_state_supported, eval_source);
            queue_reliability_transition(
                &mut updates,
                &subject,
                Some(monitored_oid),
                Reliability::CONFIGURATION_ERROR,
                EventEnrollmentReliabilityCause::Configuration,
                CapturedReferencedValue::Unavailable,
            );
            continue;
        }

        match read_monitored_reliability(monitored_obj) {
            MonitoredReliability::ConfigurationError => {
                queue_evaluation_ownership(
                    &mut updates,
                    *oid,
                    eval_state_supported,
                    reset_evaluation_state,
                    eval_source,
                    monitored_reference,
                );
                queue_reliability_transition(
                    &mut updates,
                    &subject,
                    Some(monitored_oid),
                    Reliability::CONFIGURATION_ERROR,
                    EventEnrollmentReliabilityCause::Configuration,
                    CapturedReferencedValue::NotEvaluated,
                );
                continue;
            }
            MonitoredReliability::ObservationUnavailable => {
                queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
                continue;
            }
            MonitoredReliability::Value(reliability)
                if reliability != Reliability::NO_FAULT_DETECTED =>
            {
                queue_evaluation_ownership(
                    &mut updates,
                    *oid,
                    eval_state_supported,
                    reset_evaluation_state,
                    eval_source,
                    monitored_reference,
                );
                queue_reliability_transition(
                    &mut updates,
                    &subject,
                    Some(monitored_oid),
                    Reliability::MONITORED_OBJECT_FAULT,
                    EventEnrollmentReliabilityCause::MonitoredObject,
                    CapturedReferencedValue::NotEvaluated,
                );
                continue;
            }
            MonitoredReliability::Absent | MonitoredReliability::Value(_) => {}
        }

        let mut monitored_value = None;
        if matches!(fault_algorithm, SupportedFaultAlgorithm::OutOfRange { .. }) {
            match monitored_obj.read_property(monitored_prop, monitored.array_index) {
                Ok(value) => monitored_value = Some(value),
                Err(error)
                    if monitored.array_index.is_some() && invalid_indexed_target_error(&error) =>
                {
                    queue_invalid_reference(&mut updates, *oid, eval_state_supported, eval_source);
                    queue_reliability_transition(
                        &mut updates,
                        &subject,
                        Some(monitored_oid),
                        Reliability::CONFIGURATION_ERROR,
                        EventEnrollmentReliabilityCause::Configuration,
                        CapturedReferencedValue::Unavailable,
                    );
                    continue;
                }
                Err(_) => {
                    queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
                    continue;
                }
            }
        }

        match evaluate_fault_algorithm(db, &fault_algorithm, monitored_value.as_ref()) {
            FaultAlgorithmEvaluation::ConfigurationError => {
                queue_evaluation_ownership(
                    &mut updates,
                    *oid,
                    eval_state_supported,
                    reset_evaluation_state,
                    eval_source,
                    monitored_reference,
                );
                queue_reliability_transition(
                    &mut updates,
                    &subject,
                    Some(monitored_oid),
                    Reliability::CONFIGURATION_ERROR,
                    EventEnrollmentReliabilityCause::Configuration,
                    CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
                );
                continue;
            }
            FaultAlgorithmEvaluation::ObservationUnavailable => {
                queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
                continue;
            }
            FaultAlgorithmEvaluation::Fault(reliability) => {
                queue_evaluation_ownership(
                    &mut updates,
                    *oid,
                    eval_state_supported,
                    reset_evaluation_state,
                    eval_source,
                    monitored_reference,
                );
                queue_reliability_transition(
                    &mut updates,
                    &subject,
                    Some(monitored_oid),
                    reliability,
                    EventEnrollmentReliabilityCause::FaultAlgorithm,
                    CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
                );
                continue;
            }
            FaultAlgorithmEvaluation::Healthy => {}
        }

        if current_reliability != Reliability::NO_FAULT_DETECTED
            || current_state == EventState::FAULT
        {
            eval_state = EventEnrollmentEvalState::default();
            queue_evaluation_ownership(
                &mut updates,
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
                &mut updates,
                &subject,
                Some(monitored_oid),
                Reliability::NO_FAULT_DETECTED,
                cause,
                CapturedReferencedValue::from_evaluated(monitored_value.as_ref()),
            );
            continue;
        }

        let monitored_value = match monitored_value {
            Some(value) => value,
            None => match monitored_obj.read_property(monitored_prop, monitored.array_index) {
                Ok(value) => value,
                Err(error)
                    if monitored.array_index.is_some() && invalid_indexed_target_error(&error) =>
                {
                    queue_invalid_reference(&mut updates, *oid, eval_state_supported, eval_source);
                    queue_reliability_transition(
                        &mut updates,
                        &subject,
                        Some(monitored_oid),
                        Reliability::CONFIGURATION_ERROR,
                        EventEnrollmentReliabilityCause::Configuration,
                        CapturedReferencedValue::Unavailable,
                    );
                    continue;
                }
                Err(_) => {
                    queue_observation_gap(&mut updates, *oid, eval_state_supported, eval_source);
                    continue;
                }
            },
        };

        queue_evaluation_ownership(
            &mut updates,
            *oid,
            eval_state_supported,
            reset_evaluation_state,
            eval_source,
            monitored_reference,
        );

        // The effective normal-direction delay: the EE object's read arm
        // applies the Clause 13.3 fallback (Time_Delay_Normal absent → the
        // Event_Parameters Time_Delay), so this read IS pTimeDelayNormal.
        let normal_delay =
            match enrollment.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None) {
                Ok(PropertyValue::Unsigned(v)) => u32::try_from(v).unwrap_or(u32::MAX),
                _ => 0,
            };

        // An object whose trait implementation predates the private state
        // channel evaluates without a durable countdown or COV baseline.
        let mut eval_state_dirty = false;

        // Event_Algorithm_Inhibit (Clauses 12.12.18 and 13.2.2.1.5) sets the
        // algorithm aside; fault handling above has already run. It is read
        // before any exit that the algorithm's own inputs could take (a
        // parameter fingerprint, a monitored value of the wrong datatype, a
        // setpoint gap), so an offnormal enrollment returns to NORMAL whenever
        // its monitored value can be read. See `inhibited_arm`.
        let inhibited = matches!(
            enrollment.read_property(PropertyIdentifier::EVENT_ALGORITHM_INHIBIT, None),
            Ok(PropertyValue::Boolean(true))
        );

        // A parameter (or reference) change mid-pending cancels the countdown
        // and re-gates from the current parameters; no partial countdown is
        // resumed. An inhibited pass seeds no countdown, so it needs none.
        let fingerprint =
            match params_fingerprint(&params, normal_delay as u64, event_type, &monitored) {
                Ok(fingerprint) => Some(fingerprint),
                Err(_) if inhibited => None,
                Err(_) => {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                }
            };
        if eval_state
            .pending
            .as_ref()
            .is_some_and(|p| Some(p.params_fingerprint) != fingerprint)
        {
            eval_state.pending = None;
            if eval_state_supported {
                updates
                    .entry(*oid)
                    .or_default()
                    .set_eval_state(eval_state.clone());
            }
        }

        let mut projection_setpoint = None;
        let (time_delay, arm) = match &params {
            _ if inhibited => {
                // The forced NORMAL's notification wants the setpoint too,
                // when it can be read; a gap only leaves it out.
                if let BACnetEventParameter::FloatingLimit {
                    setpoint_reference, ..
                } = &params
                    && let SetpointRead::Value(setpoint) =
                        read_setpoint(db, local_device, setpoint_reference)
                {
                    projection_setpoint = Some(setpoint);
                }
                (0, inhibited_arm(&params, &monitored_value, current_state))
            }
            BACnetEventParameter::OutOfRange {
                high_limit,
                low_limit,
                deadband,
                time_delay,
            } => {
                let Some(val) = extract_real(&monitored_value) else {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                };
                (
                    *time_delay,
                    eval_out_of_range_struct(
                        *low_limit,
                        *high_limit,
                        *deadband,
                        val,
                        current_state,
                    ),
                )
            }
            BACnetEventParameter::FloatingLimit {
                setpoint_reference,
                low_diff_limit,
                high_diff_limit,
                deadband,
                time_delay,
            } => {
                let Some(val) = extract_real(&monitored_value) else {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                };
                let setpoint = match read_setpoint(db, local_device, setpoint_reference) {
                    SetpointRead::Value(value) => value,
                    SetpointRead::Unusable => {
                        queue_pending_cancellation(
                            &mut updates,
                            *oid,
                            eval_state_supported,
                            &mut eval_state,
                        );
                        continue;
                    }
                    SetpointRead::Transient => {
                        queue_observation_gap(
                            &mut updates,
                            *oid,
                            eval_state_supported,
                            eval_source,
                        );
                        continue;
                    }
                };
                projection_setpoint = Some(setpoint);
                (
                    *time_delay,
                    eval_floating_limit_struct(
                        setpoint,
                        *high_diff_limit,
                        *low_diff_limit,
                        *deadband,
                        val,
                        current_state,
                    ),
                )
            }
            BACnetEventParameter::ChangeOfState {
                list_of_values,
                time_delay,
            } => {
                let Some(val) = extract_property_state_value(&monitored_value) else {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                };
                (
                    *time_delay,
                    eval_change_of_state_struct(
                        list_of_values,
                        val,
                        current_state,
                        eval_state.last_offnormal_value,
                    ),
                )
            }
            BACnetEventParameter::ChangeOfBitstring {
                bitmask,
                list_of_values,
                time_delay,
            } => {
                let Some(bits) = extract_bitstring(&monitored_value) else {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                };
                (
                    *time_delay,
                    eval_change_of_bitstring_struct(bitmask, list_of_values, &bits, current_state),
                )
            }
            BACnetEventParameter::ChangeOfValue {
                criteria,
                time_delay,
            } => {
                let Some(eval) = eval_change_of_value_struct(
                    criteria,
                    &monitored_value,
                    eval_state.cov_baseline.as_ref(),
                    current_state,
                ) else {
                    queue_pending_cancellation(
                        &mut updates,
                        *oid,
                        eval_state_supported,
                        &mut eval_state,
                    );
                    continue;
                };
                (*time_delay, eval)
            }
            // Legacy raw-octet writes are stored under the sentinel tag 0xFF
            // with the private little-endian payload — only those route to
            // the byte-layout evaluator. A framed write of an alternative
            // under an unassigned tag decodes to `Opaque` too, but its
            // payload is a context-tagged TLV body: feeding tag bytes to the
            // LE evaluator would reinterpret them as IEEE-754 limits and
            // fabricate spurious HIGH/LOW_LIMIT transitions from a peer's
            // configuration. Such alternatives are preserved for read-back
            // but never evaluated.
            //
            // The LE layouts carry no Time_Delay field: delay 0 keeps this
            // path's historical immediate-transition behavior.
            BACnetEventParameter::Opaque { tag: 0xFF, data } => (
                0,
                eval_legacy_le_arm(data, &monitored_value, current_state, event_type),
            ),
            // `none` [20] has no offnormal states (#1868).
            BACnetEventParameter::None => (0, eval_none_arm(current_state)),
            BACnetEventParameter::Opaque { .. } => continue,
            // Extended [9] and the algorithms this evaluator doesn't run
            // (COMMAND_FAILURE, CHANGE_OF_LIFE_SAFETY, BUFFER_READY,
            // UNSIGNED_RANGE, ACCESS_EVENT, the typed out-of-range forms,
            // CHANGE_OF_CHARACTERSTRING, CHANGE_OF_STATUS_FLAGS,
            // CHANGE_OF_DISCRETE_VALUE, CHANGE_OF_TIMER) produce no
            // transition here, as when they were kept as `Opaque` (#2090).
            _ => continue,
        };

        let ArmEvaluation {
            indication,
            establish_baseline,
        } = arm;
        if let Some(baseline) = establish_baseline {
            eval_state.cov_baseline = Some(baseline);
            eval_state_dirty = true;
        }

        let Some(ind) = indication else {
            // No condition true — or the condition that seeded the countdown
            // reverted: cancel any pending transition without firing.
            if eval_state.pending.is_some() {
                eval_state.pending = None;
                eval_state_dirty = true;
            }
            if eval_state_dirty && eval_state_supported {
                updates.entry(*oid).or_default().set_eval_state(eval_state);
            }
            continue;
        };
        // Direction-selected delay: pTimeDelay toward OFFNORMAL states,
        // pTimeDelayNormal (already the fallback-composed effective value)
        // toward NORMAL — the Clause 13.3 split, mirroring the intrinsic
        // detectors' `delay_toward` without sharing code across the boundary.
        let delay = if inhibited {
            0
        } else if ind.target == EventState::NORMAL {
            normal_delay
        } else {
            time_delay
        };

        let fired = if let (false, Some(fingerprint)) = (delay == 0, fingerprint) {
            let mut fire = None;
            match &mut eval_state.pending {
                // In flight to the same target under the same condition: the
                // countdown advances; a redundant qualifying observation does
                // NOT re-seed it (Clause 13.3 counts the delay from when the
                // condition began to hold, the same rule the intrinsic
                // detectors document at `OutOfRangeDetector::probe`).
                Some(p) if p.state == ind.target && p.condition == ind.condition => {
                    p.remaining = p.remaining.saturating_sub(1);
                    eval_state_dirty = true;
                    if p.remaining == 0 {
                        fire = Some(ind);
                    }
                }
                // No countdown, or the condition's target changed mid-delay:
                // (re-)seed with the current target's direction-appropriate
                // delay, converted from seconds with never-fire-early ceiling
                // semantics.
                _ => {
                    eval_state.pending = Some(EventEnrollmentPending {
                        state: ind.target,
                        remaining: passes_for_delay(delay, interval_secs),
                        condition: ind.condition,
                        params_fingerprint: fingerprint,
                    });
                    eval_state_dirty = true;
                }
            }
            if fire.is_some() {
                eval_state.pending = None;
            }
            fire
        } else {
            // A transition that fires at once leaves no countdown behind,
            // whichever target it was counting toward (an inhibited pass
            // always fires at once).
            if eval_state.pending.take().is_some() {
                eval_state_dirty = true;
            }
            Some(ind)
        };

        let Some(fired) = fired else {
            if eval_state_dirty && eval_state_supported {
                updates.entry(*oid).or_default().set_eval_state(eval_state);
            }
            continue;
        };

        // The transition fired. Its side-effect state lands now: the COV
        // baseline becomes the value at the indicated NORMAL transition
        // (Clause 13.3.3), and a fired OFFNORMAL records its causing value
        // for CHANGE_OF_STATE condition (c).
        if let Some(baseline) = fired.new_baseline {
            eval_state.cov_baseline = Some(baseline);
            eval_state_dirty = true;
        }
        if let Some(causing) = fired.offnormal_value {
            eval_state.last_offnormal_value = Some(causing);
            eval_state_dirty = true;
        }

        // `Event_Enable` governs distribution only (Clause 12.12). The
        // transition is recorded either way; the flag rides along so the
        // notification pipeline can suppress the send (#127).
        let transition_bit = EventTransition::for_target_state(fired.target).bit_mask();
        let distribute = event_enable.contains(transition_bit);
        let ack_required = ack_required_for_transition(db, enrollment, transition_bit);

        let update = updates.entry(*oid).or_default();
        if eval_state_dirty && eval_state_supported {
            update.set_eval_state(eval_state);
        }
        update.fire(FiredTransition {
            monitored_oid,
            event_type,
            from: current_state,
            to: fired.target,
            distribute,
            ack_required,
            projection: EventEnrollmentProjectionSnapshot::new(
                monitored.object_identifier,
                monitored.property_identifier,
                monitored.array_index,
                monitored_value,
                params,
                capture_status_flags(monitored_obj),
                projection_setpoint,
            ),
        });
    }

    apply_updates_for_delivery(db, &oids, updates, &database_eval_sources)
}

#[cfg(test)]
use tests::helpers::{evaluate_event_enrollments, evaluate_event_enrollments_report};

#[cfg(test)]
mod tests;
