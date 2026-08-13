//! Per-algorithm evaluation for the Event Enrollment evaluator.
//!
//! Split out of `mod.rs` so both files stay under the 700-LOC cap: this file
//! holds the pure evaluation functions — the byte-layout evaluators retained
//! for the legacy `Opaque` path, and the structured evaluators that consume
//! [`BACnetEventParameter`] fields directly — while `mod.rs` holds the
//! database driver and delay gating.
//!
//! Every structured evaluator returns an `Option<`[`Indication`]`>`: either
//! the algorithm *indicated a transition*, or it did not ("If no condition
//! evaluates to true, then no transition shall be indicated", Clause 13.3's
//! common introduction). The pre-#163 code folded both into one `EventState`
//! return and discarded each algorithm's `time_delay` entirely.

use bacnet_types::constructed::{BACnetPropertyStates, ChangeOfValueCriteria};
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::PropertyValue;

/// A transition the event algorithm indicated on this pass.
///
/// The driver still owes the delay gate (Clause 13.3's "for pTimeDelay" /
/// "for pTimeDelayNormal") before this becomes a fired transition; an
/// indication only asserts that an algorithm condition is currently true.
pub(crate) struct Indication {
    /// The specific BACnetEventState the algorithm returned. Stored verbatim
    /// in `Event_State` when the transition fires (Clause 13.2.2.1.4: "it is
    /// not acceptable to set Event_State to OFFNORMAL when the returned value
    /// is HIGH_LIMIT").
    pub target: EventState,
    /// Identity of the indicating condition, consumed by the driver's pending
    /// countdown — see `EventEnrollmentPending::condition` in bacnet-objects.
    /// CHANGE_OF_STATE discriminates by matched alarm value (Clause 13.3.2
    /// keys its delays on *which* value is equal: "remains equal to that
    /// value for pTimeDelay"); CHANGE_OF_BITSTRING by a hash of the masked
    /// monitored bytes; algorithms whose delay gates a threshold condition
    /// (OUT_OF_RANGE, FLOATING_LIMIT, CHANGE_OF_VALUE) use `0`.
    pub condition: u64,
    /// CHANGE_OF_STATE only: the matched alarm value, recorded as the value
    /// that caused the transition to OFFNORMAL when the transition *fires*
    /// (drives condition (c) on later passes).
    pub offnormal_value: Option<u32>,
}

impl Indication {
    /// An indication carrying only a target and condition identity.
    pub(crate) fn plain(target: EventState, condition: u64) -> Self {
        Self {
            target,
            condition,
            offnormal_value: None,
        }
    }
}

// ---- Event parameter encoding helpers ----

/// Encode OUT_OF_RANGE parameters: `[high_limit: f32 LE][low_limit: f32 LE][deadband: f32 LE]`
pub fn encode_out_of_range_params(high_limit: f32, low_limit: f32, deadband: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&high_limit.to_le_bytes());
    buf.extend_from_slice(&low_limit.to_le_bytes());
    buf.extend_from_slice(&deadband.to_le_bytes());
    buf
}

/// Encode FLOATING_LIMIT parameters:
/// `[setpoint: f32 LE][high_diff: f32 LE][low_diff: f32 LE][deadband: f32 LE]`
pub fn encode_floating_limit_params(
    setpoint: f32,
    high_diff_limit: f32,
    low_diff_limit: f32,
    deadband: f32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&setpoint.to_le_bytes());
    buf.extend_from_slice(&high_diff_limit.to_le_bytes());
    buf.extend_from_slice(&low_diff_limit.to_le_bytes());
    buf.extend_from_slice(&deadband.to_le_bytes());
    buf
}

/// Encode CHANGE_OF_STATE parameters: `[count: u32 LE][alarm_values: u32 LE ...]`
pub fn encode_change_of_state_params(alarm_values: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + alarm_values.len() * 4);
    buf.extend_from_slice(&(alarm_values.len() as u32).to_le_bytes());
    for &v in alarm_values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// Encode CHANGE_OF_VALUE parameters: `[increment: f32 LE]`
pub fn encode_change_of_value_params(increment: f32) -> Vec<u8> {
    increment.to_le_bytes().to_vec()
}

/// Encode CHANGE_OF_BITSTRING parameters:
/// `[mask_len: u32 LE][mask_bytes ...][alarm_bits ...]`
pub fn encode_change_of_bitstring_params(mask: &[u8], alarm_bits: &[u8]) -> Vec<u8> {
    let len = mask.len().min(alarm_bits.len());
    let mut buf = Vec::with_capacity(4 + len * 2);
    buf.extend_from_slice(&(len as u32).to_le_bytes());
    buf.extend_from_slice(&mask[..len]);
    buf.extend_from_slice(&alarm_bits[..len]);
    buf
}

// ---- Algorithm evaluation (byte layout; also the legacy Opaque path) ----

/// Evaluate the OUT_OF_RANGE algorithm.
///
/// Compares a real present_value against high/low limits with deadband hysteresis.
fn eval_out_of_range(params: &[u8], value: f32, current: EventState) -> EventState {
    if params.len() < 12 {
        return current;
    }
    let high_limit = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let low_limit = f32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    let deadband = f32::from_le_bytes([params[8], params[9], params[10], params[11]]);

    match current {
        s if s == EventState::NORMAL => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value < low_limit {
                EventState::LOW_LIMIT
            } else {
                EventState::NORMAL
            }
        }
        s if s == EventState::HIGH_LIMIT => {
            if value < low_limit {
                EventState::LOW_LIMIT
            } else if value < high_limit - deadband {
                EventState::NORMAL
            } else {
                EventState::HIGH_LIMIT
            }
        }
        s if s == EventState::LOW_LIMIT => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value > low_limit + deadband {
                EventState::NORMAL
            } else {
                EventState::LOW_LIMIT
            }
        }
        _ => current,
    }
}

/// Evaluate the FLOATING_LIMIT algorithm.
///
/// Compares a real present_value against a setpoint +/- differential limits
/// with deadband hysteresis.
fn eval_floating_limit(params: &[u8], value: f32, current: EventState) -> EventState {
    if params.len() < 16 {
        return current;
    }
    let setpoint = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let high_diff = f32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    let low_diff = f32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let deadband = f32::from_le_bytes([params[12], params[13], params[14], params[15]]);

    let high_limit = setpoint + high_diff;
    let low_limit = setpoint - low_diff;

    match current {
        s if s == EventState::NORMAL => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value < low_limit {
                EventState::LOW_LIMIT
            } else {
                EventState::NORMAL
            }
        }
        s if s == EventState::HIGH_LIMIT => {
            if value < low_limit {
                EventState::LOW_LIMIT
            } else if value < high_limit - deadband {
                EventState::NORMAL
            } else {
                EventState::HIGH_LIMIT
            }
        }
        s if s == EventState::LOW_LIMIT => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value > low_limit + deadband {
                EventState::NORMAL
            } else {
                EventState::LOW_LIMIT
            }
        }
        _ => current,
    }
}

/// Evaluate the CHANGE_OF_STATE algorithm (byte layout).
///
/// OFFNORMAL if the value matches any alarm value, otherwise NORMAL.
fn eval_change_of_state(params: &[u8], value: u32, _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let count = u32::from_le_bytes([params[0], params[1], params[2], params[3]]) as usize;
    let needed = 4usize.saturating_add(count.saturating_mul(4));
    if params.len() < needed {
        return EventState::NORMAL;
    }
    for i in 0..count {
        let offset = 4 + i * 4;
        let alarm_val = u32::from_le_bytes([
            params[offset],
            params[offset + 1],
            params[offset + 2],
            params[offset + 3],
        ]);
        if value == alarm_val {
            return EventState::OFFNORMAL;
        }
    }
    EventState::NORMAL
}

/// Evaluate the CHANGE_OF_BITSTRING algorithm (byte layout).
///
/// Applies a mask to the monitored bitstring and compares against the alarm pattern.
fn eval_change_of_bitstring(params: &[u8], value_bits: &[u8], _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let mask_len = u32::from_le_bytes([params[0], params[1], params[2], params[3]]) as usize;
    let needed = 4usize.saturating_add(mask_len.saturating_mul(2));
    if params.len() < needed {
        return EventState::NORMAL;
    }

    let mask = &params[4..4 + mask_len];
    let alarm_bits = &params[4 + mask_len..4 + 2 * mask_len];

    for i in 0..mask_len {
        let monitored_byte = value_bits.get(i).copied().unwrap_or(0);
        if (monitored_byte & mask[i]) != (alarm_bits[i] & mask[i]) {
            return EventState::NORMAL;
        }
    }
    EventState::OFFNORMAL
}

/// Evaluate the CHANGE_OF_VALUE algorithm (byte layout, legacy path only).
///
/// OFFNORMAL if |current_value| >= increment, otherwise NORMAL. Retained
/// verbatim for `Opaque` event parameters written by pre-structured clients;
/// the structured arm has its own implementation.
fn eval_change_of_value(params: &[u8], value: f32, _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let increment = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    if increment <= 0.0 || !increment.is_finite() {
        return EventState::NORMAL;
    }
    if value.abs() >= increment {
        EventState::OFFNORMAL
    } else {
        EventState::NORMAL
    }
}

// ---- Structured evaluation (consumes BACnetEventParameter fields) ----

/// Structured OUT_OF_RANGE evaluation with explicit limits and deadband.
///
/// OUT_OF_RANGE defines no same-state condition (Clause 13.3.6 (a)–(h) are
/// all state-changing, and persistence inside the band is the common
/// introduction's "no condition evaluates to true"), so the hysteresis
/// result yields an indication only when it differs from the current state.
pub(crate) fn eval_out_of_range_struct(
    low_limit: f32,
    high_limit: f32,
    deadband: f32,
    value: f32,
    current: EventState,
) -> Option<Indication> {
    let new_state = eval_out_of_range(
        &encode_out_of_range_params(high_limit, low_limit, deadband),
        value,
        current,
    );
    (new_state != current).then(|| Indication::plain(new_state, 0))
}

/// Structured FLOATING_LIMIT evaluation with an explicit setpoint.
///
/// Same indication shape as OUT_OF_RANGE: Clause 13.3.5's conditions are all
/// state-changing, so persistence inside the deadband yields no indication.
pub(crate) fn eval_floating_limit_struct(
    setpoint: f32,
    high_diff_limit: f32,
    low_diff_limit: f32,
    deadband: f32,
    value: f32,
    current: EventState,
) -> Option<Indication> {
    let new_state = eval_floating_limit(
        &encode_floating_limit_params(setpoint, high_diff_limit, low_diff_limit, deadband),
        value,
        current,
    );
    (new_state != current).then(|| Indication::plain(new_state, 0))
}

/// Whether a [`BACnetPropertyStates`] payload equals the monitored discrete
/// value.
fn property_state_matches(state: &BACnetPropertyStates, value: u32) -> bool {
    use bacnet_types::constructed::BACnetPropertyStates as S;
    match state {
        S::BooleanValue(v) => value == u32::from(*v),
        S::BinaryValue(v) => value == *v,
        S::EventType(v) => value == *v,
        S::Polarity(v) => value == *v,
        S::ProgramChange(v) => value == *v,
        S::ProgramState(v) => value == *v,
        S::ReasonForHalt(v) => value == *v,
        S::Reliability(v) => value == *v,
        S::State(v) => value == *v,
        S::SystemStatus(v) => value == *v,
        S::Units(v) => value == *v,
        S::UnsignedValue(v) => value == *v,
        S::LifeSafetyMode(v) => value == *v,
        S::LifeSafetyState(v) => value == *v,
        S::DoorAlarmState(v) => value == *v,
        S::Action(v) => value == *v,
        S::DoorSecuredStatus(v) => value == *v,
        S::DoorStatus(v) => value == *v,
        S::DoorValue(v) => value == *v,
        S::LiftCarDirection(v) => value == *v,
        S::LiftCarDoorCommand(v) => value == *v,
        S::TimerState(v) => value == *v,
        S::TimerTransition(v) => value == *v,
        S::Other { .. } => false,
    }
}

/// Structured CHANGE_OF_STATE evaluation against a list of alarm values.
///
/// Implements Clause 13.3.2's conditions in the presented order:
/// (a) NORMAL + monitored value equals an alarm value → OFFNORMAL;
/// (b) OFFNORMAL + value equals no alarm value → NORMAL;
/// (c) OFFNORMAL + value equals a *different* alarm value than the one that
///     caused the last OFFNORMAL transition → re-indicate OFFNORMAL.
///
/// Condition (c) is marked "Optional:" in the standard. It is implemented
/// here because without it an enrollment whose monitored value moves
/// between listed alarm values would sit silently OFFNORMAL — and Clause
/// 13.2.2.1.4 requires the transition actions even for an
/// OFFNORMAL→OFFNORMAL result (issue #166). `last_offnormal_value` is what
/// distinguishes (c) from an unchanged alarm condition; when it is unknown
/// (an `Event_State` seeded by the test/setup helper rather than by
/// evaluation), (c) declines to indicate rather than fabricating a re-entry
/// every pass.
pub(crate) fn eval_change_of_state_struct(
    alarm_values: &[BACnetPropertyStates],
    value: u32,
    current: EventState,
    last_offnormal_value: Option<u32>,
) -> Option<Indication> {
    let matched = alarm_values
        .iter()
        .any(|s| property_state_matches(s, value));
    let offnormal = || Indication {
        target: EventState::OFFNORMAL,
        condition: value as u64,
        offnormal_value: Some(value),
    };
    if current == EventState::NORMAL && matched {
        Some(offnormal())
    } else if current == EventState::OFFNORMAL && !matched {
        Some(Indication::plain(EventState::NORMAL, 0))
    } else if current == EventState::OFFNORMAL
        && matched
        && last_offnormal_value.is_some_and(|caused| caused != value)
    {
        Some(offnormal())
    } else {
        None
    }
}

/// FNV-1a over the masked monitored bytes — the condition identity for a
/// CHANGE_OF_BITSTRING (a) indication.
fn masked_value_hash(mask: &[u8], value_bits: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, &m) in mask.iter().enumerate() {
        let b = value_bits.get(i).copied().unwrap_or(0) & m;
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Structured CHANGE_OF_BITSTRING evaluation against a bitmask and alarm values.
///
/// Clause 13.3.1 conditions (a) and (b): NORMAL + masked value equals an
/// alarm value → OFFNORMAL; OFFNORMAL + masked value equals none → NORMAL.
/// Condition (c) ("Optional:") is deliberately NOT implemented: it keys on a
/// masked value "different from the value that caused the last transition to
/// OFFNORMAL", no such baseline is retained for bitstrings, and guessing
/// would re-indicate on every poll while a value sits unchanged in an alarm
/// pattern — the failure mode issue #166 documented for an unguarded pass.
pub(crate) fn eval_change_of_bitstring_struct(
    bitmask: &(u8, Vec<u8>),
    list_of_values: &[(u8, Vec<u8>)],
    value_bits: &[u8],
    current: EventState,
) -> Option<Indication> {
    // OFFNORMAL if the masked monitored bits match any alarm pattern.
    let mask = &bitmask.1;
    let mut matched = false;
    for alarm in list_of_values {
        let alarm_bits = &alarm.1;
        let len = mask.len().min(alarm_bits.len()).min(value_bits.len());
        let mut this_matches = len > 0;
        for i in 0..len {
            if (value_bits[i] & mask[i]) != (alarm_bits[i] & mask[i]) {
                this_matches = false;
                break;
            }
        }
        if this_matches {
            matched = true;
            break;
        }
    }
    if current == EventState::NORMAL && matched {
        Some(Indication::plain(
            EventState::OFFNORMAL,
            masked_value_hash(mask, value_bits),
        ))
    } else if current == EventState::OFFNORMAL && !matched {
        Some(Indication::plain(EventState::NORMAL, 0))
    } else {
        None
    }
}

/// Structured CHANGE_OF_VALUE evaluation against a `cov-criteria`.
///
/// For a `bitmask` criterion the monitored value is a bitstring and the
/// algorithm reports OFFNORMAL when any masked bit is set; for a
/// `referenced-property-increment` criterion the monitored value is a real
/// and the algorithm reports OFFNORMAL when `|value| >= increment`. Returns
/// `None` when the monitored value is the wrong type for the criterion, so
/// the caller can skip the enrollment rather than spuriously transitioning
/// to `NORMAL`.
///
/// Known limitation (#137): the comparison has no change *baseline*, so a
/// stable large magnitude keeps indicating OFFNORMAL — and Clause 13.3.3's
/// only true condition (change against the value at the last NORMAL
/// indication, targeting NORMAL per Figure 13-10) is unimplemented.
pub(crate) fn eval_change_of_value_struct(
    criteria: &ChangeOfValueCriteria,
    monitored_value: &PropertyValue,
    current: EventState,
) -> Option<Option<Indication>> {
    use bacnet_types::constructed::ChangeOfValueCriteria as C;
    match criteria {
        C::Bitmask { data, .. } => {
            let bits = extract_bitstring(monitored_value)?;
            let mut state = EventState::NORMAL;
            for i in 0..data.len().min(bits.len()) {
                if (bits[i] & data[i]) != 0 {
                    state = EventState::OFFNORMAL;
                    break;
                }
            }
            Some((state != current).then(|| Indication::plain(state, 0)))
        }
        C::ReferencedPropertyIncrement(increment) => extract_real(monitored_value).map(|v| {
            let state = eval_change_of_value(&increment.to_le_bytes(), v, EventState::NORMAL);
            (state != current).then(|| Indication::plain(state, 0))
        }),
    }
}

/// Legacy little-endian fallback for `Opaque` event parameters.
///
/// Used when an enrollment's `Event_Parameters` could not be decoded into a
/// structured alternative (e.g. raw octets written by an older client that
/// used the private little-endian byte layouts). The algorithm is inferred
/// from the enrollment's `Event_Type`, and the original byte-oriented
/// evaluators consume the opaque payload. Returns `current` (no transition)
/// when the `Event_Type` does not name a known evaluator or the monitored
/// value is the wrong type.
///
/// These byte layouts predate delay honoring and carry no Time_Delay field:
/// the driver gates this path with `time_delay == 0`, preserving the
/// immediate-transition behavior such configurations have always had.
fn eval_legacy_le(
    data: &[u8],
    monitored_value: &PropertyValue,
    current: EventState,
    event_type: EventType,
) -> EventState {
    if event_type == EventType::OUT_OF_RANGE {
        extract_real(monitored_value)
            .map(|v| eval_out_of_range(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::FLOATING_LIMIT {
        extract_real(monitored_value)
            .map(|v| eval_floating_limit(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_STATE {
        extract_enumerated(monitored_value)
            .map(|v| eval_change_of_state(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_BITSTRING {
        extract_bitstring(monitored_value)
            .map(|bits| eval_change_of_bitstring(data, &bits, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_VALUE {
        extract_real(monitored_value)
            .map(|v| eval_change_of_value(data, v, current))
            .unwrap_or(current)
    } else {
        current
    }
}

/// Route the legacy `Opaque` path through the indication model: the byte
/// evaluators return a bare state, so "result differs from current" is the
/// only indication signal available — identical to the pre-#163 transition
/// predicate, so legacy configurations behave exactly as before.
pub(crate) fn eval_legacy_le_arm(
    data: &[u8],
    monitored_value: &PropertyValue,
    current: EventState,
    event_type: EventType,
) -> Option<Indication> {
    let new_state = eval_legacy_le(data, monitored_value, current, event_type);
    (new_state != current).then(|| Indication::plain(new_state, 0))
}

/// Extract a real (f32) value from a PropertyValue.
pub(crate) fn extract_real(pv: &PropertyValue) -> Option<f32> {
    match pv {
        PropertyValue::Real(v) => Some(*v),
        PropertyValue::Double(v) => Some(*v as f32),
        PropertyValue::Unsigned(v) => Some(*v as f32),
        PropertyValue::Signed(v) => Some(*v as f32),
        _ => None,
    }
}

/// Extract an enumerated (u32) value from a PropertyValue.
pub(crate) fn extract_enumerated(pv: &PropertyValue) -> Option<u32> {
    match pv {
        PropertyValue::Enumerated(v) => Some(*v),
        PropertyValue::Unsigned(v) => Some(*v as u32),
        _ => None,
    }
}

/// Extract bitstring bytes from a PropertyValue.
pub(crate) fn extract_bitstring(pv: &PropertyValue) -> Option<Vec<u8>> {
    match pv {
        PropertyValue::BitString { data, .. } => Some(data.clone()),
        _ => None,
    }
}
