//! The event algorithm's part of one enrollment's evaluation: the delays,
//! Event_Algorithm_Inhibit and the countdown's identity, one function per
//! algorithm that indicates a transition from the monitored value, then the
//! time-delay countdown and the transition that fires. As in
//! [`evaluate`](super::evaluate), a step that settles the enrollment's
//! outcome for this pass stages it in `updates` and returns `None`.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use bacnet_objects::event::EventTransition;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentPending};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria,
};
use bacnet_types::enums::{EventState, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::algorithms::{
    ArmEvaluation, eval_change_of_bitstring_struct, eval_change_of_state_struct,
    eval_change_of_value_struct, eval_floating_limit_struct, eval_legacy_le_arm, eval_none_arm,
    eval_out_of_range_struct, extract_bitstring, extract_property_state_value, extract_real,
    inhibited_arm,
};
use super::commit::{EnrollmentUpdate, FiredTransition};
use super::evaluate::{EnrollmentInputs, MonitoredTarget};
use super::reference::params_fingerprint;
use super::support::*;
use crate::event::{EventEnrollmentProjectionSnapshot, capture_status_flags};

/// What [`select_algorithm`] hands [`count_down_and_fire`]: the delays and
/// countdown identity the indication is gated by, what the algorithm
/// indicated, and the values a fired transition's notification carries.
pub(super) struct Selected {
    normal_delay: u32,
    inhibited: bool,
    fingerprint: Option<u64>,
    projection_setpoint: Option<f32>,
    time_delay: u32,
    arm: ArmEvaluation,
    params: BACnetEventParameter,
    monitored_value: PropertyValue,
}

/// What one algorithm's evaluation reads, and where it stages the outcome
/// when the monitored value or its setpoint can't be evaluated.
struct AlgorithmScope<'s, 'a> {
    inputs: &'s EnrollmentInputs<'a>,
    updates: &'s mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    eval_state: &'s mut EventEnrollmentEvalState,
    monitored_value: &'s PropertyValue,
}

/// Read the delays and Event_Algorithm_Inhibit, drop a countdown the
/// parameters no longer match, and run the event algorithm the parameters
/// name. `None` when the algorithm can't run on this value, or produces no
/// transition here.
pub(super) fn select_algorithm(
    inputs: &EnrollmentInputs<'_>,
    target: &MonitoredTarget,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    eval_state: &mut EventEnrollmentEvalState,
    params: BACnetEventParameter,
    monitored_value: PropertyValue,
) -> Option<Selected> {
    let EnrollmentInputs {
        db,
        oid,
        enrollment,
        local_device,
        eval_state_supported,
        event_type,
        current_state,
        ..
    } = *inputs;
    let MonitoredTarget { monitored, .. } = *target;

    // The effective normal-direction delay: the EE object's read arm
    // applies the Clause 13.3 fallback (Time_Delay_Normal absent → the
    // Event_Parameters Time_Delay), so this read IS pTimeDelayNormal.
    let normal_delay = match enrollment.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None) {
        Ok(PropertyValue::Unsigned(v)) => u32::try_from(v).unwrap_or(u32::MAX),
        _ => 0,
    };

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
    let fingerprint = match params_fingerprint(&params, normal_delay as u64, event_type, &monitored)
    {
        Ok(fingerprint) => Some(fingerprint),
        Err(_) if inhibited => None,
        Err(_) => {
            queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
            return None;
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
    let scope = AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value: &monitored_value,
    };
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
        } => evaluate_out_of_range(scope, high_limit, low_limit, deadband, time_delay)?,
        BACnetEventParameter::FloatingLimit {
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
            time_delay,
        } => evaluate_floating_limit(
            scope,
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
            time_delay,
            &mut projection_setpoint,
        )?,
        BACnetEventParameter::ChangeOfState {
            list_of_values,
            time_delay,
        } => evaluate_change_of_state(scope, list_of_values, time_delay)?,
        BACnetEventParameter::ChangeOfBitstring {
            bitmask,
            list_of_values,
            time_delay,
        } => evaluate_change_of_bitstring(scope, bitmask, list_of_values, time_delay)?,
        BACnetEventParameter::ChangeOfValue {
            criteria,
            time_delay,
        } => evaluate_change_of_value(scope, criteria, time_delay)?,
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
        BACnetEventParameter::Opaque { .. } => return None,
        // Extended [9] and the algorithms this evaluator doesn't run
        // (COMMAND_FAILURE, CHANGE_OF_LIFE_SAFETY, BUFFER_READY,
        // UNSIGNED_RANGE, ACCESS_EVENT, the typed out-of-range forms,
        // CHANGE_OF_CHARACTERSTRING, CHANGE_OF_STATUS_FLAGS,
        // CHANGE_OF_DISCRETE_VALUE, CHANGE_OF_TIMER) produce no
        // transition here, as when they were kept as `Opaque` (#2090).
        _ => return None,
    };
    Some(Selected {
        normal_delay,
        inhibited,
        fingerprint,
        projection_setpoint,
        time_delay,
        arm,
        params,
        monitored_value,
    })
}

/// OUT_OF_RANGE over a REAL monitored value.
fn evaluate_out_of_range(
    scope: AlgorithmScope<'_, '_>,
    high_limit: &f32,
    low_limit: &f32,
    deadband: &f32,
    time_delay: &u32,
) -> Option<(u32, ArmEvaluation)> {
    let AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value,
    } = scope;
    let EnrollmentInputs {
        oid,
        eval_state_supported,
        current_state,
        ..
    } = *inputs;
    let Some(val) = extract_real(monitored_value) else {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    };
    Some((
        *time_delay,
        eval_out_of_range_struct(*low_limit, *high_limit, *deadband, val, current_state),
    ))
}

/// FLOATING_LIMIT over a REAL monitored value and the setpoint its
/// reference reads, which `projection_setpoint` keeps for the notification.
fn evaluate_floating_limit(
    scope: AlgorithmScope<'_, '_>,
    setpoint_reference: &BACnetDeviceObjectPropertyReference,
    low_diff_limit: &f32,
    high_diff_limit: &f32,
    deadband: &f32,
    time_delay: &u32,
    projection_setpoint: &mut Option<f32>,
) -> Option<(u32, ArmEvaluation)> {
    let AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value,
    } = scope;
    let EnrollmentInputs {
        db,
        oid,
        local_device,
        eval_state_supported,
        eval_source,
        current_state,
        ..
    } = *inputs;
    let Some(val) = extract_real(monitored_value) else {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    };
    let setpoint = match read_setpoint(db, local_device, setpoint_reference) {
        SetpointRead::Value(value) => value,
        SetpointRead::Unusable => {
            queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
            return None;
        }
        SetpointRead::Transient => {
            queue_observation_gap(updates, *oid, eval_state_supported, eval_source);
            return None;
        }
    };
    *projection_setpoint = Some(setpoint);
    Some((
        *time_delay,
        eval_floating_limit_struct(
            setpoint,
            *high_diff_limit,
            *low_diff_limit,
            *deadband,
            val,
            current_state,
        ),
    ))
}

/// CHANGE_OF_STATE over a monitored value with a property state, against
/// the value that last caused an offnormal transition.
fn evaluate_change_of_state(
    scope: AlgorithmScope<'_, '_>,
    list_of_values: &[BACnetPropertyStates],
    time_delay: &u32,
) -> Option<(u32, ArmEvaluation)> {
    let AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value,
    } = scope;
    let EnrollmentInputs {
        oid,
        eval_state_supported,
        current_state,
        ..
    } = *inputs;
    let Some(val) = extract_property_state_value(monitored_value) else {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    };
    Some((
        *time_delay,
        eval_change_of_state_struct(
            list_of_values,
            val,
            current_state,
            eval_state.last_offnormal_value,
        ),
    ))
}

/// CHANGE_OF_BITSTRING over a BIT STRING monitored value.
fn evaluate_change_of_bitstring(
    scope: AlgorithmScope<'_, '_>,
    bitmask: &(u8, Vec<u8>),
    list_of_values: &[(u8, Vec<u8>)],
    time_delay: &u32,
) -> Option<(u32, ArmEvaluation)> {
    let AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value,
    } = scope;
    let EnrollmentInputs {
        oid,
        eval_state_supported,
        current_state,
        ..
    } = *inputs;
    let Some(bits) = extract_bitstring(monitored_value) else {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    };
    Some((
        *time_delay,
        eval_change_of_bitstring_struct(bitmask, list_of_values, &bits, current_state),
    ))
}

/// CHANGE_OF_VALUE against the baseline the private state holds.
fn evaluate_change_of_value(
    scope: AlgorithmScope<'_, '_>,
    criteria: &ChangeOfValueCriteria,
    time_delay: &u32,
) -> Option<(u32, ArmEvaluation)> {
    let AlgorithmScope {
        inputs,
        updates,
        eval_state,
        monitored_value,
    } = scope;
    let EnrollmentInputs {
        oid,
        eval_state_supported,
        current_state,
        ..
    } = *inputs;
    let Some(eval) = eval_change_of_value_struct(
        criteria,
        monitored_value,
        eval_state.cov_baseline.as_ref(),
        current_state,
    ) else {
        queue_pending_cancellation(updates, *oid, eval_state_supported, eval_state);
        return None;
    };
    Some((*time_delay, eval))
}

/// Gate the indication by its time delay, counting a pending transition
/// down by one pass, and stage the private state and the transition that
/// fires.
pub(super) fn count_down_and_fire(
    inputs: &EnrollmentInputs<'_>,
    target: &MonitoredTarget,
    updates: &mut BTreeMap<ObjectIdentifier, EnrollmentUpdate>,
    mut eval_state: EventEnrollmentEvalState,
    selected: Selected,
    monitored_obj: &dyn BACnetObject,
) {
    let EnrollmentInputs {
        db,
        oid,
        enrollment,
        interval_secs,
        eval_state_supported,
        event_type,
        current_state,
        event_enable,
        ..
    } = *inputs;
    let MonitoredTarget {
        monitored,
        monitored_oid,
        ..
    } = *target;
    let Selected {
        normal_delay,
        inhibited,
        fingerprint,
        projection_setpoint,
        time_delay,
        arm,
        params,
        monitored_value,
    } = selected;

    // An object whose trait implementation predates the private state
    // channel evaluates without a durable countdown or COV baseline.
    let mut eval_state_dirty = false;

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
        return;
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
        return;
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
