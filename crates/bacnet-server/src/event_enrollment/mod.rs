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
//! The pending countdown lives in the EE object (in-memory only) and advances
//! once per evaluation pass of this evaluator — a "tick" is one
//! `event_enrollment_task` interval (`event_enrollment_interval_secs`, 10s by
//! default), so a delay of N suppresses the transition for N evaluation
//! passes. This mirrors the intrinsic detectors' probe/tick semantics
//! (`bacnet_objects::event`, #120/#225) — condition reverted cancels, same
//! target never restarts, changed target re-seeds — without sharing code
//! across the objects/server boundary.
//!
//! Known limitation (#166): an indicated transition identical to the current
//! state is still dropped — Clause 13.2.2.1.4's same-state transition actions
//! and the CHANGE_OF_VALUE baseline (#137) are follow-on work; likewise no
//! notification is sent here (#127, tranche E) and `Event_Time_Stamps` /
//! `Event_Message_Texts` stay unmodeled (#264).

mod algorithms;

pub use algorithms::{
    encode_change_of_bitstring_params, encode_change_of_state_params,
    encode_change_of_value_params, encode_floating_limit_params, encode_out_of_range_params,
};

use algorithms::{
    eval_change_of_bitstring_struct, eval_change_of_state_struct, eval_change_of_value_struct,
    eval_floating_limit_struct, eval_legacy_le_arm, eval_out_of_range_struct, extract_bitstring,
    extract_enumerated, extract_real, Indication,
};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{EventStateChange, EventTransition};
use bacnet_objects::event_enrollment::EventEnrollmentPending;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// A state transition detected during event enrollment evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnrollmentTransition {
    /// The EventEnrollment object that detected the transition.
    pub enrollment_oid: ObjectIdentifier,
    /// The monitored object whose property triggered the transition.
    pub monitored_oid: ObjectIdentifier,
    /// The detected state change.
    pub change: EventStateChange,
    /// The event type that was evaluated.
    pub event_type: EventType,
    /// Whether `Event_Enable` permits distributing a notification for this
    /// transition. The transition is reported and `Event_State` persisted
    /// either way; a cleared bit suppresses only the outbound notification
    /// (ASHRAE 135-2020 Clause 12.12).
    pub distribute: bool,
}

/// Read the setpoint referenced by a FLOATING_LIMIT enrollment.
///
/// Returns the referenced property's real value, or `None` if the reference is
/// remote or unreadable.
fn read_setpoint(
    db: &ObjectDatabase,
    reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
) -> Option<f32> {
    // Remote-device setpoint references are not resolvable from a local DB.
    if reference.device_identifier.is_some() {
        return None;
    }
    let obj = db.get(&reference.object_identifier)?;
    let prop = PropertyIdentifier::from_raw(reference.property_identifier);
    extract_real(
        &obj.read_property(prop, reference.property_array_index)
            .ok()?,
    )
}

/// Read the object_property_reference from an EventEnrollment object.
///
/// Returns (monitored_object_id, monitored_property_id) if valid.
fn read_object_property_ref(
    enrollment: &dyn BACnetObject,
) -> Option<(ObjectIdentifier, PropertyIdentifier)> {
    match enrollment.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None) {
        Ok(PropertyValue::List(ref items)) if items.len() >= 2 => {
            let obj_id = match &items[0] {
                PropertyValue::ObjectIdentifier(oid) => *oid,
                _ => return None,
            };
            let prop_id = match &items[1] {
                PropertyValue::Unsigned(v) => PropertyIdentifier::from_raw(*v as u32),
                _ => return None,
            };
            Some((obj_id, prop_id))
        }
        _ => None,
    }
}

/// Fingerprint the configuration a pending countdown was gated under: the
/// framed `Event_Parameters` encoding, the effective normal-direction delay,
/// and the configured event type. A mismatch with
/// [`EventEnrollmentPending::params_fingerprint`] cancels the in-flight
/// countdown and re-gates from the current parameters — the pinned behavior
/// for a mid-pending parameter change (the evaluator re-reads parameters
/// every pass, so the change is observed on the pass after the write).
fn params_fingerprint(
    params: &BACnetEventParameter,
    normal_delay: u64,
    event_type_raw: u32,
) -> u64 {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, params);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in buf
        .iter()
        .copied()
        .chain(normal_delay.to_le_bytes())
        .chain(event_type_raw.to_le_bytes())
    {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// What phase 1 decided for one enrollment, applied under a mutable borrow in
/// phase 2 (phase 1 holds immutable database borrows to read monitored
/// objects, so all mutation is deferred).
struct EnrollmentUpdate {
    /// Evaluation state to write back — the pending countdown — when it
    /// changed this pass, even with no transition.
    eval_state: Option<bacnet_objects::event_enrollment::EventEnrollmentEvalState>,
    /// A transition that fired this pass.
    fired: Option<FiredTransition>,
}

struct FiredTransition {
    monitored_oid: ObjectIdentifier,
    event_type_raw: u32,
    from: EventState,
    to: EventState,
    distribute: bool,
}

impl EnrollmentUpdate {
    fn eval_state_only(
        eval_state: bacnet_objects::event_enrollment::EventEnrollmentEvalState,
    ) -> Self {
        Self {
            eval_state: Some(eval_state),
            fired: None,
        }
    }
}

/// Evaluate all EventEnrollment objects in the database.
///
/// For each active enrollment, reads the monitored property, evaluates the
/// configured algorithm, applies the Time_Delay / Time_Delay_Normal
/// countdown, and returns the transitions that fired.
pub fn evaluate_event_enrollments(db: &mut ObjectDatabase) -> Vec<EventEnrollmentTransition> {
    let oids = db.find_by_type(ObjectType::EVENT_ENROLLMENT);

    let mut updates: Vec<(ObjectIdentifier, EnrollmentUpdate)> = Vec::new();

    for oid in &oids {
        let Some(enrollment) = db.get(oid) else {
            continue;
        };

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

        let params = match enrollment.read_property(PropertyIdentifier::EVENT_PARAMETERS, None) {
            // Framed wire form (the EventEnrollment object's read arm emits
            // full ASN.1 CHOICE framing).
            Ok(PropertyValue::ApplicationData(bytes)) => {
                match bacnet_encoding::constructed::decode_event_parameter(&bytes, 0) {
                    Ok((ep, _)) => ep,
                    // Malformed framed value: nothing to evaluate.
                    Err(_) => continue,
                }
            }
            // Legacy flat application-tagged form (downstream/custom object
            // types that have not migrated to the framed read arm).
            Ok(v) => match BACnetEventParameter::decode(&v) {
                Ok(ep) => ep,
                // Malformed structured value: nothing to evaluate.
                Err(_) => continue,
            },
            // Missing/unreadable Event_Parameters: nothing to evaluate.
            Err(_) => continue,
        };

        // The effective normal-direction delay: the EE object's read arm
        // applies the Clause 13.3 fallback (Time_Delay_Normal absent → the
        // Event_Parameters Time_Delay), so this read IS pTimeDelayNormal.
        // Unreadable (custom object without the property) degrades to 0,
        // the pre-#163 immediate-transition behavior.
        let normal_delay =
            match enrollment.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None) {
                Ok(PropertyValue::Unsigned(v)) => u32::try_from(v).unwrap_or(u32::MAX),
                _ => 0,
            };

        // Per-enrollment evaluation state owned by the object. An object whose
        // trait impl predates the channel (a downstream custom EE type)
        // reports `None`: evaluation still runs, but with no durable pending
        // countdown — a nonzero delay then re-seeds every pass and never
        // fires, so delays are effectively unsupported for such objects
        // (TD=0 configurations behave exactly as before for them).
        let eval_state_supported = enrollment.enrollment_eval_state_internal().is_some();
        let mut eval_state = enrollment
            .enrollment_eval_state_internal()
            .unwrap_or_default();
        let mut eval_state_dirty = false;

        // A parameter change mid-pending cancels the countdown and re-gates
        // from the current parameters; no partial countdown is resumed.
        let fingerprint = params_fingerprint(&params, normal_delay as u64, event_type_raw);
        if eval_state
            .pending
            .as_ref()
            .is_some_and(|p| p.params_fingerprint != fingerprint)
        {
            eval_state.pending = None;
            eval_state_dirty = true;
        }

        let Some((monitored_oid, monitored_prop)) = read_object_property_ref(enrollment) else {
            continue;
        };

        let Some(monitored_obj) = db.get(&monitored_oid) else {
            continue;
        };
        let monitored_value = match monitored_obj.read_property(monitored_prop, None) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = EventType::from_raw(event_type_raw);
        let (time_delay, indication) = match &params {
            BACnetEventParameter::OutOfRange {
                high_limit,
                low_limit,
                deadband,
                time_delay,
            } => {
                let Some(val) = extract_real(&monitored_value) else {
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
                let Some(setpoint) = read_setpoint(db, setpoint_reference) else {
                    continue;
                };
                let Some(val) = extract_real(&monitored_value) else {
                    continue;
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
                let Some(val) = extract_enumerated(&monitored_value) else {
                    continue;
                };
                (
                    *time_delay,
                    eval_change_of_state_struct(list_of_values, val, current_state),
                )
            }
            BACnetEventParameter::ChangeOfBitstring {
                bitmask,
                list_of_values,
                time_delay,
            } => {
                let Some(bits) = extract_bitstring(&monitored_value) else {
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
                let Some(ind) =
                    eval_change_of_value_struct(criteria, &monitored_value, current_state)
                else {
                    continue;
                };
                (*time_delay, ind)
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

        let Some(ind) = indication else {
            // No condition true — or the condition that seeded the countdown
            // reverted: cancel any pending transition without firing.
            if eval_state.pending.is_some() {
                eval_state.pending = None;
                eval_state_dirty = true;
            }
            if eval_state_dirty && eval_state_supported {
                updates.push((*oid, EnrollmentUpdate::eval_state_only(eval_state)));
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

        let fired: Option<Indication> = if delay == 0 {
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
                // delay.
                _ => {
                    eval_state.pending = Some(EventEnrollmentPending {
                        state: ind.target,
                        remaining: delay,
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
                updates.push((*oid, EnrollmentUpdate::eval_state_only(eval_state)));
            }
            continue;
        };

        // `Event_Enable` governs distribution only (Clause 12.12). The
        // transition is recorded either way; the flag rides along so the
        // notification pipeline can suppress the send (#127).
        let transition_bit = EventTransition::for_target_state(fired.target).bit_mask();
        let distribute = event_enable & transition_bit != 0;

        updates.push((
            *oid,
            EnrollmentUpdate {
                eval_state: (eval_state_dirty && eval_state_supported).then_some(eval_state),
                fired: Some(FiredTransition {
                    monitored_oid,
                    event_type_raw,
                    from: current_state,
                    to: fired.target,
                    distribute,
                }),
            },
        ));
    }

    let mut transitions = Vec::new();
    for (oid, update) in updates {
        let Some(obj) = db.get_mut(&oid) else {
            continue;
        };
        if let Some(fired) = update.fired {
            // Persist the transition through the internal lifecycle path, not
            // the network `write_property(EVENT_STATE, …)` route. `Event_State`
            // is algorithmically derived (ASHRAE 135-2020 Clause 12.12) and
            // read-only over the network, so the evaluator reaches the field
            // via `set_event_state_internal` (issue #130).
            if obj.set_event_state_internal(fired.to).is_err() {
                continue;
            }
            if let Some(state) = update.eval_state {
                let _ = obj.set_enrollment_eval_state_internal(state);
            }
            transitions.push(EventEnrollmentTransition {
                enrollment_oid: oid,
                monitored_oid: fired.monitored_oid,
                change: EventStateChange {
                    from: fired.from,
                    to: fired.to,
                },
                event_type: EventType::from_raw(fired.event_type_raw),
                distribute: fired.distribute,
            });
        } else if let Some(state) = update.eval_state {
            let _ = obj.set_enrollment_eval_state_internal(state);
        }
    }

    transitions
}

#[cfg(test)]
mod tests;
