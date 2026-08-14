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
//! `Event_Time_Stamps` / `Event_Message_Texts` are not modeled on the EE
//! object (tranche E, #264), and no notification is sent (#127, tranche E) —
//! the lifecycle task only logs the returned transitions.

mod algorithms;

pub use algorithms::{
    encode_change_of_bitstring_params, encode_change_of_state_params,
    encode_change_of_value_params, encode_floating_limit_params, encode_out_of_range_params,
};

use algorithms::{
    eval_change_of_bitstring_struct, eval_change_of_state_struct, eval_change_of_value_struct,
    eval_floating_limit_struct, eval_legacy_le_arm, eval_out_of_range_struct, extract_bitstring,
    extract_enumerated, extract_real, ArmEvaluation,
};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{EventStateChange, EventTransition};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentPending};
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
/// Returns the monitored object, property, and optional device qualifier.
fn read_object_property_ref(
    enrollment: &dyn BACnetObject,
) -> Result<
    Option<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<ObjectIdentifier>,
    )>,
    (),
> {
    match enrollment.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None) {
        Ok(PropertyValue::List(ref items)) if (2..=4).contains(&items.len()) => {
            let obj_id = match &items[0] {
                PropertyValue::ObjectIdentifier(oid) => *oid,
                _ => return Ok(None),
            };
            let PropertyValue::Unsigned(prop_id) = &items[1] else {
                return Ok(None);
            };
            let Ok(prop_id) = u32::try_from(*prop_id) else {
                return Ok(None);
            };
            if prop_id > 0x3F_FFFF {
                return Ok(None);
            }
            let prop_id = PropertyIdentifier::from_raw(prop_id);
            match items.get(2) {
                None | Some(PropertyValue::Null | PropertyValue::Unsigned(_)) => {}
                Some(_) => return Ok(None),
            }
            let device_id = match items.get(3) {
                None | Some(PropertyValue::Null) => None,
                Some(PropertyValue::ObjectIdentifier(oid)) => Some(*oid),
                Some(_) => return Ok(None),
            };
            Ok(Some((obj_id, prop_id, device_id)))
        }
        Ok(_) => Ok(None),
        Err(_) => Err(()),
    }
}

/// Convert a seconds delay to pending passes with never-fire-early ceiling
/// semantics: `ceil(delay_secs / interval_secs)`. At the default 10s
/// interval a 5s delay seeds 1 pass (fires when that pass elapses, ~10s
/// later); a 15s delay seeds 2 (~20s). Callers never pass `delay_secs == 0`
/// — a zero delay fires without seeding.
fn passes_for_delay(delay_secs: u32, interval_secs: u64) -> u32 {
    let passes = (delay_secs as u64).div_ceil(interval_secs.max(1));
    u32::try_from(passes).unwrap_or(u32::MAX)
}

/// Fingerprint the configuration a pending countdown was gated under: the
/// framed `Event_Parameters` encoding (which includes any algorithm-carried
/// references, e.g. FLOATING_LIMIT's `setpoint_reference`), the effective
/// normal-direction delay, the configured event type, and the monitored
/// object+property the enrollment's `Object_Property_Reference` resolves to.
/// A mismatch with [`EventEnrollmentPending::params_fingerprint`] cancels
/// the in-flight countdown and re-gates from the current parameters — the
/// pinned behavior for a mid-pending retarget or parameter change (the
/// evaluator re-reads them every pass, so the change is observed on the pass
/// after the write).
fn params_fingerprint(
    params: &BACnetEventParameter,
    normal_delay: u64,
    event_type_raw: u32,
    monitored: &(ObjectIdentifier, PropertyIdentifier),
) -> u64 {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, params);
    let (monitored_oid, monitored_prop) = monitored;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in buf
        .iter()
        .copied()
        .chain(normal_delay.to_le_bytes())
        .chain(event_type_raw.to_le_bytes())
        .chain(monitored_oid.encode())
        .chain(monitored_prop.to_raw().to_le_bytes())
    {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Resolve whether the Notification Class referenced by an enrollment
/// requires acknowledgment of `transition_bit`.
///
/// Clause 13.2.3: "Whether or not an acknowledgment is required is determined
/// by the Ack_Required property from the referenced Notification Class
/// object." A missing or unreadable Notification Class resolves to
/// not-required — the standard's fallback for an absent parameter is the
/// "otherwise it is set" half of the same sentence, which leaves
/// `Acked_Transitions` alone-equals-acknowledged rather than stranding a
/// transition permanently unacknowledged for want of a class object.
fn ack_required_for_transition(
    db: &ObjectDatabase,
    enrollment: &dyn BACnetObject,
    transition_bit: u8,
) -> bool {
    let Ok(PropertyValue::Unsigned(nc_instance)) =
        enrollment.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
    else {
        return false;
    };
    let Ok(nc_oid) = ObjectIdentifier::new(
        ObjectType::NOTIFICATION_CLASS,
        u32::try_from(nc_instance).unwrap_or(u32::MAX),
    ) else {
        return false;
    };
    let Some(nc) = db.get(&nc_oid) else {
        return false;
    };
    match nc.read_property(PropertyIdentifier::ACK_REQUIRED, None) {
        Ok(PropertyValue::BitString { data, .. }) => {
            bacnet_types::bitstring::unpack_octet(&data, 3) & transition_bit != 0
        }
        _ => false,
    }
}

/// What phase 1 decided for one enrollment, applied under a mutable borrow in
/// phase 2 (phase 1 holds immutable database borrows to read monitored
/// objects, so all mutation is deferred).
struct EnrollmentUpdate {
    /// Evaluation state to write back — pending countdown and/or baselines —
    /// when it changed this pass, even with no transition.
    eval_state: Option<EventEnrollmentEvalState>,
    /// A transition that fired this pass, with everything the transition
    /// actions need.
    fired: Option<FiredTransition>,
}

struct FiredTransition {
    monitored_oid: ObjectIdentifier,
    event_type_raw: u32,
    from: EventState,
    to: EventState,
    distribute: bool,
    transition_bit: u8,
    ack_required: bool,
}

impl EnrollmentUpdate {
    fn eval_state_only(eval_state: EventEnrollmentEvalState) -> Self {
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
    let interval_secs = interval_secs.max(1);
    let oids = db.find_by_type(ObjectType::EVENT_ENROLLMENT);
    // A qualified reference can identify self only when the containing Device
    // object is unambiguous. Unqualified references remain local regardless.
    let device_oids = db.find_by_type(ObjectType::DEVICE);
    let local_device_oid = match device_oids.as_slice() {
        [oid] if oid.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE => Some(*oid),
        _ => None,
    };

    let mut updates: Vec<(ObjectIdentifier, EnrollmentUpdate)> = Vec::new();

    for oid in &oids {
        let Some(enrollment) = db.get(oid) else {
            continue;
        };

        // A property read failure is transient and retains evaluation state.
        // Any successfully read invalid or unsupported-device reference clears
        // state before unrelated properties can short-circuit this pass.
        let eval_state_supported = enrollment.enrollment_eval_state_internal().is_some();
        let mut eval_state = enrollment
            .enrollment_eval_state_internal()
            .unwrap_or_default();
        let reference = match read_object_property_ref(enrollment) {
            Ok(reference) => reference,
            Err(()) => continue,
        };
        let Some((monitored_oid, monitored_prop, _)) = reference.filter(|(_, _, device_oid)| {
            device_oid.is_none_or(|oid| Some(oid) == local_device_oid)
        }) else {
            if eval_state_supported && eval_state != EventEnrollmentEvalState::default() {
                updates.push((
                    *oid,
                    EnrollmentUpdate::eval_state_only(EventEnrollmentEvalState::default()),
                ));
            }
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

        // An object whose trait implementation predates the private state
        // channel evaluates without a durable countdown or COV baseline.
        let mut eval_state_dirty = false;

        // A parameter (or reference) change mid-pending cancels the countdown
        // and re-gates from the current parameters; no partial countdown is
        // resumed. The cancellation is flushed BEFORE any later exit — a
        // dropped write-back here is what let a params round-trip A→B→A
        // resume a stale countdown.
        let fingerprint = params_fingerprint(
            &params,
            normal_delay as u64,
            event_type_raw,
            &(monitored_oid, monitored_prop),
        );
        if eval_state
            .pending
            .as_ref()
            .is_some_and(|p| p.params_fingerprint != fingerprint)
        {
            eval_state.pending = None;
            eval_state_dirty = false;
            if eval_state_supported {
                updates.push((*oid, EnrollmentUpdate::eval_state_only(eval_state.clone())));
            }
        }

        let Some(monitored_obj) = db.get(&monitored_oid) else {
            continue;
        };
        let monitored_value = match monitored_obj.read_property(monitored_prop, None) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = EventType::from_raw(event_type_raw);
        let (time_delay, arm) = match &params {
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
                updates.push((*oid, EnrollmentUpdate::eval_state_only(eval_state)));
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
                    transition_bit,
                    ack_required,
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
            // via `set_event_state_internal` (issue #130). The SPECIFIC
            // returned state is stored — 13.2.2.1.4 forbids collapsing
            // HIGH_LIMIT/LOW_LIMIT to OFFNORMAL — and the actions run for
            // same-state transitions too (`from == to`).
            if obj.set_event_state_internal(fired.to).is_err() {
                continue;
            }
            // 13.2.2.1.4's fourth action, alarm-acknowledgment half (13.2.3):
            // with the transition's Ack_Required bit set, the corresponding
            // Acked_Transitions bit is cleared (ack now owed); otherwise it is
            // set. The notification-distribution half is tranche E's #127.
            let _ = obj.set_acked_transitions_internal(fired.transition_bit, !fired.ack_required);
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
