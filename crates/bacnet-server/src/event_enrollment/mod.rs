//! Event Enrollment algorithmic evaluation.
//!
//! Unlike intrinsic reporting (built into object types), Event Enrollment is a
//! separate object that monitors another object's property and evaluates an
//! algorithm against it.
//!
//! Supported algorithms: OUT_OF_RANGE, FLOATING_LIMIT, CHANGE_OF_STATE,
//! CHANGE_OF_BITSTRING, CHANGE_OF_VALUE.
//!
//! Delay model (#163): `Event_Parameters.Time_Delay` (pTimeDelay, Table 12-15)
//! gates every indicated transition into an OFFNORMAL state; the EE object's
//! optional `Time_Delay_Normal` property (pTimeDelayNormal, Table 12-14 O —
//! falling back to pTimeDelay per Clause 13.3) gates transitions to NORMAL.
//! Both delays are SECONDS in the standard (e.g. 13.3.1: "the time, in
//! seconds, that the offnormal conditions must exist"), and this evaluator
//! keeps them in seconds: the pending countdown (owned by the EE object,
//! in-memory only) is seeded with `ceil(delay_secs / interval_secs)` —
//! never-fire-early ceiling semantics, so at the default 10s
//! `event_enrollment_interval_secs` a 5s delay fires on the second pass
//! (~10s elapsed), not after five passes (~50s) — and advances once per
//! evaluation pass. Semantics otherwise mirror the intrinsic detectors'
//! probe/tick (`bacnet_objects::event`, #120/#225) — condition reverted
//! cancels, same target never restarts, changed target re-seeds — without
//! sharing code across the objects/server boundary.
//!
//! Residual countdown behavior: the interval is builder configuration, not
//! runtime-mutable, and the countdown is in-memory, so no mid-run rescale
//! exists; a restart re-evaluates from the confirmed `Event_State` and a
//! fresh `ceil` conversion, like the intrinsic detectors.
//!
//! Transition actions (#166): an *indicated* transition executes Clause
//! 13.2.2.1.4's actions even when it does not change the event state — the
//! specific returned state is stored in `Event_State`, the corresponding
//! `Acked_Transitions` bit is set/cleared per the referenced Notification
//! Class's `Ack_Required` (Clause 13.2.3), and the transition is emitted with
//! its `Event_Enable`-scoped `distribute` flag. What is NOT here, by design:
//! `Event_Time_Stamps` is committed atomically with `Event_State` and
//! `Acked_Transitions`; `Event_Message_Texts` remains absent, and no
//! notification is sent (#127) — the lifecycle task logs committed
//! transitions and internal commit failures.

mod algorithms;
mod commit;
mod fault;
mod reference;
mod support;

use std::collections::{HashMap, HashSet};

pub use algorithms::{
    encode_change_of_bitstring_params, encode_change_of_state_params,
    encode_change_of_value_params, encode_floating_limit_params, encode_out_of_range_params,
};
pub use commit::{
    EventEnrollmentDetailedEvaluationDiagnostic, EventEnrollmentDetailedEvaluationOutcome,
    EventEnrollmentDetailedEvaluationReport, EventEnrollmentDetailedEvaluationStage,
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult,
};

use algorithms::{
    eval_change_of_bitstring_struct, eval_change_of_state_struct, eval_change_of_value_struct,
    eval_floating_limit_struct, eval_legacy_le_arm, eval_out_of_range_struct, extract_bitstring,
    extract_property_state_value, extract_real, ArmEvaluation,
};
pub(crate) use commit::log_evaluation_report;
use commit::{apply_updates, EnrollmentUpdate, FiredTransition};
use fault::{
    evaluate_fault_algorithm, read_event_parameters, read_fault_algorithm,
    read_monitored_reliability, FaultAlgorithmEvaluation, MonitoredReliability,
    SupportedFaultAlgorithm,
};
#[cfg(test)]
use reference::MonitoredReference;
use reference::{params_fingerprint, read_object_property_ref};
use support::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalConfigurationReadError {
    Malformed,
    Unavailable,
}

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{EventStateChange, EventTransition};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentPending};
#[cfg(test)]
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// A state transition detected during event enrollment evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnrollmentTransition {
    /// The EventEnrollment object that detected the transition.
    pub enrollment_oid: ObjectIdentifier,
    /// The monitored object whose property triggered the transition.
    pub monitored_oid: ObjectIdentifier,
    /// The detected state change. `from == to` is a genuine same-state
    /// transition (Clause 13.2.2.1.4), emitted by CHANGE_OF_VALUE (Figure
    /// 13-10's NORMAL→NORMAL) and CHANGE_OF_STATE condition (c).
    pub change: EventStateChange,
    /// The event type that was evaluated.
    pub event_type: EventType,
    /// Whether `Event_Enable` permits distributing a notification for this
    /// transition. The transition is reported and `Event_State` persisted
    /// either way; a cleared bit suppresses only the outbound notification
    /// (ASHRAE 135-2020 Clause 12.12).
    pub distribute: bool,
}

/// Evaluate all EventEnrollment objects in the database.
///
/// For each active enrollment, reads the monitored property, evaluates the
/// configured algorithm, applies the Time_Delay / Time_Delay_Normal
/// countdown (seconds, converted with [`passes_for_delay`]), executes the
/// Clause 13.2.2.1.4 transition actions for every indicated transition that
/// fires — same-state included — and returns the fired transitions.
///
/// `interval_secs` is the driving task's evaluation period in wall-clock
/// seconds; the lifecycle passes its (clamped to >= 1)
/// `event_enrollment_interval_secs`. The conversion is never-fire-early, and
/// the pending countdown retains no residual seconds: in-memory state plus
/// builder-config interval means no mid-run rescale exists.
pub fn evaluate_event_enrollments(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> Vec<EventEnrollmentTransition> {
    evaluate_event_enrollments_report(db, interval_secs).transitions
}

/// Evaluate all EventEnrollment objects and expose legacy commit diagnostics.
///
/// This preserves the original report shape. Reliability results and typed
/// observation diagnostics are available from
/// [`evaluate_event_enrollments_detailed_report`].
pub fn evaluate_event_enrollments_report(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> EventEnrollmentEvaluationReport {
    evaluate_event_enrollments_detailed_report(db, interval_secs).into_legacy()
}

/// Evaluate all EventEnrollment objects and expose every detailed result.
///
/// Unlike [`evaluate_event_enrollments_report`], this additive API includes
/// committed Reliability results plus typed Reliability and observation
/// diagnostics. Only results whose complete object-owned commit succeeds
/// appear in `transitions` or `reliability_results`.
pub fn evaluate_event_enrollments_detailed_report(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> EventEnrollmentDetailedEvaluationReport {
    let interval_secs = interval_secs.max(1);
    let oids = db.find_by_type(ObjectType::EVENT_ENROLLMENT);
    // A qualified reference can identify self only when the containing Device
    // object is unambiguous. Unqualified references remain local regardless.
    let device_oids = db.find_by_type(ObjectType::DEVICE);
    let local_device_oid = match device_oids.as_slice() {
        [oid] if oid.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE => Some(*oid),
        _ => None,
    };

    let mut updates: HashMap<ObjectIdentifier, EnrollmentUpdate> = HashMap::new();
    let mut database_eval_sources = HashSet::new();

    for oid in &oids {
        let Some(enrollment) = db.get(oid) else {
            continue;
        };

        // A property read failure is transient and retains evaluation state.
        // An invalid reference shape or unsupported device clears state before
        // unrelated properties can short-circuit this pass.
        let eval_state_supported = enrollment.enrollment_eval_state_internal().is_some();
        let mut eval_state = enrollment
            .enrollment_eval_state_internal()
            .unwrap_or_default();
        let eval_source = match enrollment.enrollment_eval_source_internal() {
            Some(source) => Some(source),
            None if eval_state_supported => {
                database_eval_sources.insert(*oid);
                Some(db.enrollment_eval_source(oid))
            }
            None => None,
        };
        let force_state_reset = db.enrollment_eval_state_invalidated(oid);

        if let Ok(PropertyValue::Boolean(true)) =
            enrollment.read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        {
            continue;
        }

        // Clause 13.2.2.1: "If the Event_Detection_Enable property is FALSE,
        // then this state machine is not evaluated. In this case, no
        // transitions shall occur". The accompanying reset is applied by the
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

        let event_type_raw = match enrollment.read_property(PropertyIdentifier::EVENT_TYPE, None) {
            Ok(PropertyValue::Enumerated(v)) => v,
            _ => continue,
        };

        let current_state = match enrollment.read_property(PropertyIdentifier::EVENT_STATE, None) {
            Ok(PropertyValue::Enumerated(v)) => EventState::from_raw(v),
            _ => continue,
        };

        let event_enable = match enrollment.read_property(PropertyIdentifier::EVENT_ENABLE, None) {
            Ok(PropertyValue::BitString { data, .. }) => {
                bacnet_types::bitstring::unpack_octet(&data, 3)
            }
            _ => 0,
        };

        let current_reliability =
            match enrollment.read_property(PropertyIdentifier::RELIABILITY, None) {
                Ok(PropertyValue::Enumerated(value)) => Reliability::from_raw(value),
                _ => {
                    queue_observation_unavailable(&mut updates, *oid);
                    continue;
                }
            };

        // Local configuration is resolved before any target observation. A
        // malformed/missing required reference, malformed parameters, or an
        // unsupported configured fault alternative therefore wins the
        // Reliability precedence chain deterministically.
        let reference = read_object_property_ref(enrollment);
        let params = read_event_parameters(enrollment);
        let fault_algorithm = read_fault_algorithm(enrollment, local_device_oid);
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
                db,
                enrollment,
                &mut updates,
                *oid,
                monitored_oid,
                current_reliability,
                Reliability::CONFIGURATION_ERROR,
                current_state,
                event_enable,
                EventEnrollmentReliabilityCause::Configuration,
            );
            continue;
        }

        if matches!(reference, Err(LocalConfigurationReadError::Unavailable)) {
            queue_observation_unavailable(&mut updates, *oid);
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
        // local-only slice. It is not rewritten into persistent target-loss
        // policy; PR-0303 owns that behavior.
        if monitored
            .device_identifier
            .is_some_and(|device| Some(device) != local_device_oid)
        {
            queue_observation_unavailable(&mut updates, *oid);
            continue;
        }

        let monitored_oid = monitored.object_identifier;
        let monitored_prop = monitored.property_identifier;
        let monitored_reference = (monitored_oid, monitored_prop, monitored.array_index);
        // Private evaluation state belongs to one exact monitored source. A
        // nonempty ownerless state cannot be adopted safely. Defer the actual
        // setter until all required observations succeed, so an unavailable
        // observation mutates nothing.
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
            queue_observation_unavailable(&mut updates, *oid);
            continue;
        };
        if monitored.array_index.is_some() && !monitored_obj.is_array_property(monitored_prop) {
            queue_invalid_reference(&mut updates, *oid, eval_state_supported, eval_source);
            queue_reliability_transition(
                db,
                enrollment,
                &mut updates,
                *oid,
                Some(monitored_oid),
                current_reliability,
                Reliability::CONFIGURATION_ERROR,
                current_state,
                event_enable,
                EventEnrollmentReliabilityCause::Configuration,
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
                    db,
                    enrollment,
                    &mut updates,
                    *oid,
                    Some(monitored_oid),
                    current_reliability,
                    Reliability::CONFIGURATION_ERROR,
                    current_state,
                    event_enable,
                    EventEnrollmentReliabilityCause::Configuration,
                );
                continue;
            }
            MonitoredReliability::ObservationUnavailable => {
                queue_observation_unavailable(&mut updates, *oid);
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
                    db,
                    enrollment,
                    &mut updates,
                    *oid,
                    Some(monitored_oid),
                    current_reliability,
                    Reliability::MONITORED_OBJECT_FAULT,
                    current_state,
                    event_enable,
                    EventEnrollmentReliabilityCause::MonitoredObject,
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
                        db,
                        enrollment,
                        &mut updates,
                        *oid,
                        Some(monitored_oid),
                        current_reliability,
                        Reliability::CONFIGURATION_ERROR,
                        current_state,
                        event_enable,
                        EventEnrollmentReliabilityCause::Configuration,
                    );
                    continue;
                }
                Err(_) => {
                    queue_observation_unavailable(&mut updates, *oid);
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
                    db,
                    enrollment,
                    &mut updates,
                    *oid,
                    Some(monitored_oid),
                    current_reliability,
                    Reliability::CONFIGURATION_ERROR,
                    current_state,
                    event_enable,
                    EventEnrollmentReliabilityCause::Configuration,
                );
                continue;
            }
            FaultAlgorithmEvaluation::ObservationUnavailable => {
                queue_observation_unavailable(&mut updates, *oid);
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
                    db,
                    enrollment,
                    &mut updates,
                    *oid,
                    Some(monitored_oid),
                    current_reliability,
                    reliability,
                    current_state,
                    event_enable,
                    EventEnrollmentReliabilityCause::FaultAlgorithm,
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
                db,
                enrollment,
                &mut updates,
                *oid,
                Some(monitored_oid),
                current_reliability,
                Reliability::NO_FAULT_DETECTED,
                current_state,
                event_enable,
                cause,
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
                        db,
                        enrollment,
                        &mut updates,
                        *oid,
                        Some(monitored_oid),
                        current_reliability,
                        Reliability::CONFIGURATION_ERROR,
                        current_state,
                        event_enable,
                        EventEnrollmentReliabilityCause::Configuration,
                    );
                    continue;
                }
                Err(_) => {
                    queue_observation_unavailable(&mut updates, *oid);
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

        // A parameter (or reference) change mid-pending cancels the countdown
        // and re-gates from the current parameters; no partial countdown is
        // resumed.
        let Ok(fingerprint) =
            params_fingerprint(&params, normal_delay as u64, event_type_raw, &monitored)
        else {
            queue_pending_cancellation(&mut updates, *oid, eval_state_supported, &mut eval_state);
            continue;
        };
        if eval_state
            .pending
            .as_ref()
            .is_some_and(|p| p.params_fingerprint != fingerprint)
        {
            eval_state.pending = None;
            if eval_state_supported {
                updates
                    .entry(*oid)
                    .or_default()
                    .set_eval_state(eval_state.clone());
            }
        }

        let event_type = EventType::from_raw(event_type_raw);

        let (time_delay, arm) = match &params {
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
                let setpoint = match read_setpoint(db, setpoint_reference) {
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
                        queue_observation_unavailable(&mut updates, *oid);
                        continue;
                    }
                };
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
            // the byte-layout evaluator. A framed write of an UNMODELED spec
            // alternative (e.g. access-event [13]) decodes to `Opaque` too,
            // but its payload is a context-tagged TLV body: feeding tag
            // bytes to the LE evaluator would reinterpret them as IEEE-754
            // limits and fabricate spurious HIGH/LOW_LIMIT transitions from
            // a conformant peer's configuration. Unmodeled alternatives are
            // preserved for read-back but never evaluated.
            //
            // The LE layouts carry no Time_Delay field: delay 0 keeps this
            // path's historical immediate-transition behavior.
            BACnetEventParameter::Opaque { tag: 0xFF, data } => (
                0,
                eval_legacy_le_arm(data, &monitored_value, current_state, event_type),
            ),
            BACnetEventParameter::Opaque { .. } => continue,
            // Extended [9] and any other modeled-but-unmodeled-for-evaluation
            // alternatives produce no transition here.
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
        let delay = if ind.target == EventState::NORMAL {
            normal_delay
        } else {
            time_delay
        };

        let fired = if delay == 0 {
            Some(ind)
        } else {
            let mut fire = None;
            match &mut eval_state.pending {
                // In flight to the same target under the same condition: the
                // countdown advances; a redundant qualifying observation does
                // NOT re-seed it (Clause 13.2.4's debounce semantics, the same
                // rule the intrinsic detectors document at
                // `OutOfRangeDetector::probe`).
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
        let distribute = event_enable & transition_bit != 0;
        let ack_required = ack_required_for_transition(db, enrollment, transition_bit);

        let update = updates.entry(*oid).or_default();
        if eval_state_dirty && eval_state_supported {
            update.set_eval_state(eval_state);
        }
        update.fire(FiredTransition {
            monitored_oid,
            event_type_raw,
            from: current_state,
            to: fired.target,
            distribute,
            ack_required,
        });
    }

    apply_updates(db, &oids, updates, &database_eval_sources)
}

#[cfg(test)]
mod tests;
