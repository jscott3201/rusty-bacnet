//! EventEnrollment (type 9) object per ASHRAE 135-2020 Clause 12.12.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, FaultParameters,
};
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::{BACnetObject, WritePropertyRollback};

enum EventEnrollmentWriteRollback {
    Detection {
        enabled: bool,
        event_state: u32,
        acked_transitions: u8,
        evaluation: EventEnrollmentEvalState,
    },
    TimeDelayNormal(Option<u32>),
}

struct AlertEnrollmentWriteRollback {
    enabled: bool,
    event_state: u32,
    acked_transitions: u8,
}

/// A delayed Event Enrollment transition, counting down its delay.
///
/// The enrollment counterpart of the intrinsic detectors'
/// [`PendingTransition`](crate::event::PendingTransition), kept as a distinct
/// type because the driving mechanism differs: the server evaluator advances
/// `remaining` once per *evaluation pass* (the `event_enrollment_task`
/// interval, configurable via #133), whereas the intrinsic detectors tick on
/// a fixed one-second task and seed from per-write probes. Clause 13.2.4
/// semantics are shared — the observable `Event_State` holds at the confirmed
/// state while the countdown runs, a reverted condition cancels without
/// firing, and a redundant qualifying observation never re-seeds — but the
/// two implementations do not share code across the objects/server boundary.
///
/// In-memory only: like the intrinsic detectors' pending state and baselines,
/// this is not persisted; a device restart re-evaluation starts from the
/// confirmed `Event_State`, which is the same restart semantics the
/// intrinsic-reporting path ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnrollmentPending {
    /// The event state the algorithm indicated and will enter when the
    /// countdown elapses.
    pub state: EventState,
    /// Evaluation passes remaining before the transition fires; seeded with
    /// the direction-appropriate delay (pTimeDelay for offnormal targets,
    /// pTimeDelayNormal — else pTimeDelay — for NORMAL), converted from
    /// seconds by the evaluator as `ceil(delay_secs / interval_secs)`.
    pub remaining: u32,
    /// Identity of the indicating condition, per algorithm. CHANGE_OF_STATE
    /// discriminates by the matched alarm value because Clause 13.3.2
    /// conditions (a)/(c) key on *which* value the monitored value equals
    /// ("remains equal to that value for pTimeDelay"); CHANGE_OF_BITSTRING by
    /// the masked monitored bytes. Algorithms whose delay applies to the
    /// threshold condition itself (OUT_OF_RANGE, FLOATING_LIMIT,
    /// CHANGE_OF_VALUE) use `0` — the target alone identifies them.
    pub condition: u64,
    /// Fingerprint of the `Event_Parameters` (framed encoding) plus the
    /// effective `Time_Delay_Normal` in force when this countdown was seeded.
    /// The evaluator re-reads its parameters every pass; a mismatch cancels
    /// the in-flight countdown and re-gates from the current parameters —
    /// no partial countdown is resumed across a parameter change.
    pub params_fingerprint: u64,
}

/// Algorithm-side evaluation state owned by an Event Enrollment object.
///
/// Not BACnet properties: none of the three slots maps to a Clause 12.12
/// property (nor to the Table 12-14 `Time_Delay_Normal`, which is
/// configuration and lives on the object directly). Clause 13.3 assigns the
/// baseline's initialization and the countdown's existence to local matters,
/// so they are reachable only through the internal trait channel
/// ([`BACnetObject::enrollment_eval_state_internal`] /
/// [`BACnetObject::set_enrollment_eval_state_internal`]), mirroring the
/// `set_event_state_internal` precedent (issue #130).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEnrollmentEvalState {
    /// Delayed transition in flight, if any.
    pub pending: Option<EventEnrollmentPending>,
    /// CHANGE_OF_VALUE detection baseline (Clause 13.3.3: "the value of the
    /// monitored value when a transition to NORMAL is indicated shall be used
    /// in evaluation of the conditions until the next transition to NORMAL is
    /// indicated"). `None` before the first sample; the first observed value
    /// initializes it without indicating a transition ("the initialization of
    /// the value used in evaluation before the first transition to NORMAL is
    /// indicated is a local matter" — the policy chosen here).
    pub cov_baseline: Option<PropertyValue>,
    /// The monitored value that caused the last transition to OFFNORMAL, for
    /// CHANGE_OF_STATE condition (c) (Clause 13.3.2: a re-indication is
    /// indicated only when the monitored value equals an alarm value
    /// "different from the value that caused the last transition to
    /// OFFNORMAL").
    pub last_offnormal_value: Option<u32>,
}

/// BACnet EventEnrollment object.
///
/// Provides algorithmic event detection for a referenced object property.
/// The `event_parameters` are stored as a structured
/// [`BACnetEventParameter`], preserving algorithm alternatives and unknown
/// (vendor/reserved) values across a complete property round trip.
pub struct EventEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    event_type: u32,
    notify_type: u32,
    event_parameters: BACnetEventParameter,
    object_property_reference: Option<BACnetDeviceObjectPropertyReference>,
    event_state: u32,
    event_enable: u8,
    acked_transitions: u8,
    event_detection_enable: bool,
    notification_class: u32,
    fault_parameters: Option<FaultParameters>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// `Time_Delay_Normal` (property 356, Table 12-14 conformance O): the
    /// pTimeDelayNormal parameter for the object's event algorithm (Clause
    /// 12.12). `None` is the not-configured case and takes on the
    /// `Time_Delay` carried inside `event_parameters` (Table 12-15 maps
    /// `Time_Delay` to pTimeDelay for every evaluated algorithm): "If no
    /// value is available for this parameter, then it takes on the value of
    /// the pTimeDelay parameter" (Clause 13.3).
    time_delay_normal: Option<u32>,
    /// Delayed transition counting down, if any. In-memory only.
    pending: Option<EventEnrollmentPending>,
    /// CHANGE_OF_VALUE detection baseline (Clause 13.3.3). In-memory only.
    cov_baseline: Option<PropertyValue>,
    /// Monitored value that caused the last OFFNORMAL transition (Clause
    /// 13.3.2 condition (c)). In-memory only.
    last_offnormal_value: Option<u32>,
}

impl EventEnrollmentObject {
    /// Create a new EventEnrollment object.
    ///
    /// `event_type` is the BACnet EventType enumeration value.
    pub fn new(instance: u32, name: impl Into<String>, event_type: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            event_type,
            notify_type: 0,
            event_parameters: BACnetEventParameter::Opaque {
                tag: 0xFF,
                data: Vec::new(),
            },
            object_property_reference: None,
            event_state: 0,
            event_enable: 0b111,
            // Clause 12.12: "Each flag shall have the value TRUE if no event of
            // that type has ever occurred for the object." That all-TRUE value
            // is also the initial condition the detection-disabled reset
            // restores, so `RESET_ACKED_TRANSITIONS` names it once.
            acked_transitions: Self::RESET_ACKED_TRANSITIONS,
            event_detection_enable: true,
            notification_class: 0,
            fault_parameters: None,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            // Absent so the delay behavior equals the normative pTimeDelay
            // fallback until a client writes the property — never an error,
            // never a zero.
            time_delay_normal: None,
            pending: None,
            cov_baseline: None,
            last_offnormal_value: None,
        })
    }

    /// `Acked_Transitions` in its initial condition: every transition flag TRUE,
    /// meaning no event of that type has ever occurred (ASHRAE 135-2020
    /// Clause 12.12).
    const RESET_ACKED_TRANSITIONS: u8 = 0b111;

    /// Apply the reset ASHRAE 135-2020 Clause 13.2.2.1 requires while
    /// `Event_Detection_Enable` is FALSE: "no transitions shall occur,
    /// Event_State shall be set to NORMAL, and Event_Time_Stamps,
    /// Event_Message_Texts and Acked_Transitions shall be set to their
    /// respective initial conditions."
    ///
    /// `Event_Time_Stamps` and `Event_Message_Texts` are not modeled on this
    /// object yet (#264); when they are, their initial conditions belong here
    /// — X'FF' octets / sequence number 0, and the empty string respectively.
    ///
    /// The pending countdown and both baselines are cleared too: they are
    /// extensions of the same event-state-detection state machine the clause
    /// freezes ("this state machine is not evaluated"), so a stale countdown
    /// must not survive into the next enabled period and fire against a
    /// condition the object no longer observes. The intrinsic types make the
    /// same choice for their detectors (`analog/input.rs` clears
    /// `detector.pending` on the identical write). The COV baseline's
    /// initialization on re-enable is the local matter Clause 13.3.3 assigns
    /// it; clearing is consistent with the first-sample policy.
    fn apply_detection_disabled_reset(&mut self) {
        self.event_state = EventState::NORMAL.to_raw();
        self.acked_transitions = Self::RESET_ACKED_TRANSITIONS;
        self.pending = None;
        self.cov_baseline = None;
        self.last_offnormal_value = None;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the object property reference.
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetDeviceObjectPropertyReference>,
    ) {
        self.object_property_reference = reference;
    }

    /// Set the structured event parameters.
    pub fn set_event_parameters(&mut self, params: BACnetEventParameter) {
        self.event_parameters = params;
    }

    /// Set the fault parameters for this event enrollment.
    pub fn set_fault_parameters(&mut self, fp: Option<FaultParameters>) {
        self.fault_parameters = fp;
    }

    /// Set the event state (raw u32).
    ///
    /// A configuration/seeding helper, not a lifecycle path — the evaluator
    /// uses [`BACnetObject::set_event_state_internal`]. It honors the same
    /// Clause 13.2.2.1 rule: while `Event_Detection_Enable` is FALSE the object
    /// must read NORMAL, so a non-NORMAL seed is ignored rather than silently
    /// breaking the invariant. Without this the public API would offer a way
    /// around a guard the rest of the object enforces.
    pub fn set_event_state(&mut self, state: u32) {
        if !self.event_detection_enable && state != EventState::NORMAL.to_raw() {
            return;
        }
        self.event_state = state;
    }

    /// Set the notification class.
    pub fn set_notification_class(&mut self, nc: u32) {
        self.notification_class = nc;
    }

    /// Set the event enable bitmask (3 bits: TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    pub fn set_event_enable(&mut self, enable: u8) {
        self.event_enable = enable & 0x07;
    }

    /// Set `Time_Delay_Normal` (the pTimeDelayNormal parameter). `None`
    /// restores the not-configured case, which takes on the
    /// `Event_Parameters` `Time_Delay` value (Clause 13.3 fallback).
    pub fn set_time_delay_normal(&mut self, delay: Option<u32>) {
        self.time_delay_normal = delay;
    }

    /// The pTimeDelay the stored `Event_Parameters` supply: the `time_delay`
    /// field every evaluated algorithm carries (Table 12-15). Unmodeled
    /// alternatives — including the `0xFF` legacy octet layout, which has no
    /// time-delay slot — contribute zero, so their TDN fallback reads as 0
    /// and their evaluation fires immediately, exactly as they did before
    /// delay honoring existed.
    fn event_parameters_time_delay(&self) -> u32 {
        use BACnetEventParameter as P;
        match &self.event_parameters {
            P::ChangeOfBitstring { time_delay, .. }
            | P::ChangeOfState { time_delay, .. }
            | P::ChangeOfValue { time_delay, .. }
            | P::FloatingLimit { time_delay, .. }
            | P::OutOfRange { time_delay, .. } => *time_delay,
            _ => 0,
        }
    }
}

impl BACnetObject for EventEnrollmentObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::EVENT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_TYPE => {
                Ok(PropertyValue::Enumerated(self.event_type))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.notify_type))
            }
            p if p == PropertyIdentifier::EVENT_PARAMETERS => {
                let mut buf = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_event_parameter(
                    &mut buf,
                    &self.event_parameters,
                );
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                match &self.object_property_reference {
                    None => Ok(PropertyValue::Null),
                    Some(r) => Ok(PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(r.object_identifier),
                        PropertyValue::Unsigned(r.property_identifier as u64),
                        match r.property_array_index {
                            Some(idx) => PropertyValue::Unsigned(idx as u64),
                            None => PropertyValue::Null,
                        },
                        match r.device_identifier {
                            Some(dev) => PropertyValue::ObjectIdentifier(dev),
                            None => PropertyValue::Null,
                        },
                    ])),
                }
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bacnet_types::bitstring::pack_octet(self.event_enable)],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bacnet_types::bitstring::pack_octet(self.acked_transitions)],
            }),
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::FAULT_PARAMETERS => match &self.fault_parameters {
                None => Ok(PropertyValue::Null),
                Some(fp) => {
                    let mut buf = bytes::BytesMut::new();
                    bacnet_encoding::constructed::encode_fault_parameters(&mut buf, fp)?;
                    Ok(PropertyValue::ApplicationData(buf.to_vec()))
                }
            },
            p if p == PropertyIdentifier::TIME_DELAY_NORMAL => {
                // Clause 13.3: "If no value is available for this parameter,
                // then it takes on the value of the pTimeDelay parameter" —
                // the read-back of an unwritten Time_Delay_Normal is the
                // Event_Parameters Time_Delay, matching the algorithm's
                // behavior (mirrors the intrinsic types' read arm).
                Ok(PropertyValue::Unsigned(
                    self.time_delay_normal
                        .unwrap_or_else(|| self.event_parameters_time_delay())
                        as u64,
                ))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::NOTIFY_TYPE {
            // BACnetNotifyType is a closed three-value production {alarm,
            // event, ack-notification} (Clause 21); out-of-production values
            // are PROPERTY / VALUE_OUT_OF_RANGE (Clause 15.9.1.3).
            if let PropertyValue::Enumerated(v) = value {
                let named = bacnet_types::enums::NotifyType::ALL_NAMED
                    .iter()
                    .any(|&(_, n)| n.to_raw() == v);
                if !named {
                    return Err(common::value_out_of_range_error());
                }
                self.notify_type = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            if let PropertyValue::Unsigned(v) = value {
                self.notification_class = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_ENABLE {
            // BACnetEventTransitionBits is a 3-bit production (Clause 21):
            // the written BitString must declare its canonical shape.
            if let PropertyValue::BitString { unused_bits, data } = &value {
                let byte = common::check_fixed_width_bit_string(*unused_bits, data, 3)?;
                self.event_enable = bacnet_types::bitstring::unpack_octet(&[byte], 3);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                // Clause 12.12 states the disabled condition as an invariant —
                // "When this property is FALSE, Event_State shall be NORMAL" —
                // not as an action taken later. Resetting here rather than
                // leaving it to the periodic evaluator closes the window in
                // which a disabled object would still answer ReadProperty and
                // the event-summarization services with a stale alarm state.
                if !v {
                    self.apply_detection_disabled_reset();
                }
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        // EVENT_STATE is algorithmically derived (ASHRAE 135-2020 Clause 12.12)
        // and read-only over the network: a WriteProperty of EVENT_STATE falls
        // through to WRITE_ACCESS_DENIED below. The evaluator sets it through
        // the internal `set_event_state_internal` trait method instead, so the
        // network route and the internal lifecycle path no longer share an
        // access path (issue #130).
        if property == PropertyIdentifier::EVENT_PARAMETERS {
            self.event_parameters = match value {
                // Legacy raw-octet write: preserve verbatim as an Opaque value
                // using a sentinel tag (255) outside the BACnetEventParameter
                // range so it never collides with a real algorithm on decode.
                PropertyValue::OctetString(bytes) => BACnetEventParameter::Opaque {
                    tag: 0xFF,
                    data: bytes,
                },
                // Framed wire form: full ASN.1 CHOICE framing per Clause 21.
                // The CHOICE is exactly one element — trailing bytes after
                // it are rejected rather than silently swallowed (otherwise
                // the stored value reads back as only the first element).
                PropertyValue::ApplicationData(bytes) => {
                    match bacnet_encoding::constructed::decode_event_parameter(&bytes, 0) {
                        Ok((ep, consumed)) if consumed == bytes.len() => ep,
                        _ => return Err(common::invalid_data_type_error()),
                    }
                }
                // Legacy flat application-tagged form (pre-framing layout):
                // still accepted so older internal clients keep working.
                other => match BACnetEventParameter::decode(&other) {
                    Ok(ep) => ep,
                    Err(_) => return Err(common::invalid_data_type_error()),
                },
            };
            return Ok(());
        }
        if property == PropertyIdentifier::FAULT_PARAMETERS {
            self.fault_parameters = match value {
                PropertyValue::Null => None,
                PropertyValue::ApplicationData(bytes) => {
                    match bacnet_encoding::constructed::decode_fault_parameters(&bytes, 0) {
                        Ok((fp, consumed)) if consumed == bytes.len() => Some(fp),
                        _ => return Err(common::invalid_data_type_error()),
                    }
                }
                // Legacy flat application-tagged form (pre-framing layout).
                _ => match FaultParameters::decode_property_value(&value) {
                    Ok(fp) => Some(fp),
                    Err(_) => return Err(common::invalid_data_type_error()),
                },
            };
            return Ok(());
        }
        if property == PropertyIdentifier::TIME_DELAY_NORMAL {
            // Table 12-14 codes the property O, not W; accepting the write is
            // the Clause 12.1.2 implementor's option the intrinsic types
            // already exercise, and is what makes the Clause 13.3 delay
            // asymmetry commissionable on an enrollment at all.
            if let PropertyValue::Unsigned(v) = value {
                self.time_delay_normal = Some(common::u64_to_u32(v)?);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    /// Internal lifecycle path for the algorithmically-derived `Event_State`.
    ///
    /// The evaluator calls this — not `write_property(EVENT_STATE, …)` — so the
    /// network route (which rejects `EVENT_STATE`) and the internal lifecycle
    /// path are distinct (issue #130). Stores the modeled state verbatim; the
    /// only caller is the trusted server evaluator.
    ///
    /// Refuses any non-NORMAL state while `Event_Detection_Enable` is FALSE.
    /// Clause 13.2.2.1 requires that "no transitions shall occur" in that case,
    /// and the server evaluator already skips such objects — this guard makes
    /// the invariant hold by construction rather than by the caller
    /// remembering, so a future caller cannot reintroduce the violation.
    fn set_event_state_internal(&mut self, state: EventState) -> Result<(), Error> {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return Err(common::write_access_denied_error());
        }
        self.event_state = state.to_raw();
        Ok(())
    }

    /// Snapshot the enrollment evaluation state (pending countdown, COV
    /// baseline, last offnormal-causing value) for the server evaluator.
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        Some(EventEnrollmentEvalState {
            pending: self.pending.clone(),
            cov_baseline: self.cov_baseline.clone(),
            last_offnormal_value: self.last_offnormal_value,
        })
    }

    /// Store the enrollment evaluation state. Refused while
    /// `Event_Detection_Enable` is FALSE: Clause 13.2.2.1 freezes the state
    /// machine ("this state machine is not evaluated"), and the reset in the
    /// write arm has already returned these fields to their initial
    /// condition, so a write arriving while disabled can only be stale.
    fn set_enrollment_eval_state_internal(
        &mut self,
        state: EventEnrollmentEvalState,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        self.pending = state.pending;
        self.cov_baseline = state.cov_baseline;
        self.last_offnormal_value = state.last_offnormal_value;
        Ok(())
    }

    /// Acknowledge an alarm transition (the AcknowledgeAlarm service route,
    /// Clause 13.9): Clause 13.2.3 sets the bit on the acknowledgment
    /// indication — unconditional and idempotent, so a repeated ack succeeds
    /// again. A detection-DISABLED enrollment instead refuses with
    /// OBJECT/NO_ALARM_CONFIGURED, Table 13-10's "The object exists but does
    /// not support or is not configured for event generation": it can
    /// generate nothing, and Clause 12.12 keeps its `Acked_Transitions` at
    /// the initial condition, which an accepted ack would break.
    /// Out_Of_Service does not gate the ack: no clause bars acknowledging a
    /// notification already issued while the object is out of service.
    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        if !self.event_detection_enable {
            return Err(bacnet_types::error::Error::Protocol {
                class: bacnet_types::enums::ErrorClass::OBJECT.to_raw() as u32,
                code: bacnet_types::enums::ErrorCode::NO_ALARM_CONFIGURED.to_raw() as u32,
            });
        }
        self.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }

    /// Clause 13.2.3's transition-received maintenance of `Acked_Transitions`:
    /// the evaluator resolves `Ack_Required` from the referenced Notification
    /// Class object and this call applies the outcome — clear the bit when
    /// ack is required, set it otherwise. Refused while detection is
    /// disabled, the same invariant as above: "Acked_Transitions shall be
    /// equal to [its] initial condition" while FALSE.
    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: u8,
        acknowledged: bool,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        if acknowledged {
            self.acked_transitions |= transition_bit & 0x07;
        } else {
            self.acked_transitions &= !(transition_bit & 0x07);
        }
        Ok(())
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        match property {
            PropertyIdentifier::EVENT_DETECTION_ENABLE => Some(WritePropertyRollback::new(
                EventEnrollmentWriteRollback::Detection {
                    enabled: self.event_detection_enable,
                    event_state: self.event_state,
                    acked_transitions: self.acked_transitions,
                    evaluation: EventEnrollmentEvalState {
                        pending: self.pending.clone(),
                        cov_baseline: self.cov_baseline.clone(),
                        last_offnormal_value: self.last_offnormal_value,
                    },
                },
            )),
            PropertyIdentifier::TIME_DELAY_NORMAL => Some(WritePropertyRollback::new(
                EventEnrollmentWriteRollback::TimeDelayNormal(self.time_delay_normal),
            )),
            _ => None,
        }
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        match rollback.downcast::<EventEnrollmentWriteRollback>()? {
            EventEnrollmentWriteRollback::Detection {
                enabled,
                event_state,
                acked_transitions,
                evaluation,
            } => {
                self.event_detection_enable = enabled;
                self.event_state = event_state;
                self.acked_transitions = acked_transitions;
                self.pending = evaluation.pending;
                self.cov_baseline = evaluation.cov_baseline;
                self.last_offnormal_value = evaluation.last_offnormal_value;
                Ok(())
            }
            EventEnrollmentWriteRollback::TimeDelayNormal(value) => {
                self.time_delay_normal = value;
                Ok(())
            }
        }
    }

    /// Mirrors the `write_property` arms above, so PICS reports what dispatch
    /// actually accepts — with one known exception, `OBJECT_NAME`.
    ///
    /// `common::is_common_writable` includes `OBJECT_NAME`, but this object's
    /// `write_property` has no arm for it and returns `WRITE_ACCESS_DENIED`, so
    /// an Event Enrollment cannot be renamed while every core I/O/V type can
    /// (they route it through `common::write_object_name`). That is a
    /// pre-existing gap, not one this override introduces: the inherited
    /// `historical_writable_default` already returned `true` for `OBJECT_NAME`,
    /// so PICS reported the same thing before. Kept as-is to avoid a silent
    /// PICS change here; the missing rename support is tracked separately.
    ///
    /// Enumerated rather than reusing `common::is_event_property_writable`:
    /// that helper covers the intrinsic-reporting objects and includes
    /// `HIGH_LIMIT`, `LOW_LIMIT`, `DEADBAND`, `LIMIT_ENABLE` and `TIME_DELAY`,
    /// none of which an Event Enrollment accepts — it carries those inside
    /// `Event_Parameters` instead. Reusing it would over-report writability.
    /// `TIME_DELAY_NORMAL` overlaps the helper: an enrollment carries THAT
    /// one as a real (O-coded) property, per Table 12-14.
    ///
    /// `Event_Detection_Enable` is writable even though Table 12-14 codes it R
    /// rather than W: Clause 12.1.2 allows an R property to be writable "at the
    /// implementor's option unless specifically prohibited in the text
    /// describing that particular standard object's property", and Clause 12.12
    /// prohibits nothing — it only says the value "is expected" to be set
    /// during configuration, which is guidance, not a "shall". Annex K's
    /// AE-AVM-A BIBB (Table K-17) positively requires a conforming workstation
    /// to be able to *write* this property, so refusing the write would be
    /// interoperably hostile.
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        common::is_common_writable(property)
            || matches!(
                property,
                PropertyIdentifier::NOTIFY_TYPE
                    | PropertyIdentifier::NOTIFICATION_CLASS
                    | PropertyIdentifier::EVENT_ENABLE
                    | PropertyIdentifier::EVENT_DETECTION_ENABLE
                    | PropertyIdentifier::EVENT_PARAMETERS
                    | PropertyIdentifier::FAULT_PARAMETERS
                    | PropertyIdentifier::TIME_DELAY_NORMAL
            )
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_TYPE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_PARAMETERS,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::FAULT_PARAMETERS,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AlertEnrollmentObject (type 52)
// ---------------------------------------------------------------------------

/// BACnet AlertEnrollment object (type 52).
///
/// Provides alert-based event enrollment. The PRESENT_VALUE is an enumerated
/// AlertState. Supports EVENT_STATE, ACKED_TRANSITIONS,
/// EVENT_DETECTION_ENABLE, EVENT_ENABLE (3-bit), and NOTIFICATION_CLASS.
pub struct AlertEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    /// Event_State: 0 = NORMAL.
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
    /// Present value — AlertState enumeration.
    pub present_value: u32,
    /// Whether event detection is enabled.
    ///
    /// Prefer [`Self::set_event_detection_enable`] so disabling also clears
    /// stored event state. This field remains public for compatibility;
    /// property reads and internal transition hooks still enforce the
    /// disabled-state invariant after a direct assignment. Re-enable through
    /// the setter as well: a direct FALSE-to-TRUE assignment cannot run the
    /// reset and may expose state stored before the direct disable.
    pub event_detection_enable: bool,
    /// Acknowledged transitions in TO_OFFNORMAL, TO_FAULT, TO_NORMAL order.
    acked_transitions: u8,
    /// Event enable bits: 3-bit (TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    pub event_enable: u8,
    /// Notification class number.
    pub notification_class: u32,
}

impl AlertEnrollmentObject {
    /// Create a new AlertEnrollment object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ALERT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            event_state: 0, // NORMAL
            out_of_service: false,
            reliability: 0,
            present_value: 0,
            event_detection_enable: true,
            acked_transitions: 0b111,
            event_enable: 0b111,
            notification_class: 0,
        })
    }

    /// Enable or disable event detection.
    ///
    /// Disabling applies the Clause 13.2.2.1 initial conditions immediately.
    pub fn set_event_detection_enable(&mut self, enabled: bool) {
        if !enabled || !self.event_detection_enable {
            self.event_state = EventState::NORMAL.to_raw();
            self.acked_transitions = 0b111;
        }
        self.event_detection_enable = enabled;
    }
}

impl BACnetObject for AlertEnrollmentObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                if self.event_detection_enable {
                    self.event_state
                } else {
                    EventState::NORMAL.to_raw()
                },
            ));
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ALERT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bacnet_types::bitstring::pack_octet(self.event_enable)],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(if self.event_detection_enable {
                    self.event_state
                } else {
                    EventState::NORMAL.to_raw()
                }))
            }
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bacnet_types::bitstring::pack_octet(
                    if self.event_detection_enable {
                        self.acked_transitions
                    } else {
                        0b111
                    },
                )],
            }),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.set_event_detection_enable(v);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_ENABLE {
            // BACnetEventTransitionBits is a 3-bit production (Clause 21):
            // the written BitString must declare its canonical shape.
            if let PropertyValue::BitString { unused_bits, data } = &value {
                let byte = common::check_fixed_width_bit_string(*unused_bits, data, 3)?;
                self.event_enable = bacnet_types::bitstring::unpack_octet(&[byte], 3);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            if let PropertyValue::Unsigned(v) = value {
                self.notification_class = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::EVENT_DETECTION_ENABLE).then(|| {
            WritePropertyRollback::new(AlertEnrollmentWriteRollback {
                enabled: self.event_detection_enable,
                event_state: self.event_state,
                acked_transitions: self.acked_transitions,
            })
        })
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        let AlertEnrollmentWriteRollback {
            enabled,
            event_state,
            acked_transitions,
        } = rollback.downcast::<AlertEnrollmentWriteRollback>()?;
        self.event_detection_enable = enabled;
        self.event_state = event_state;
        self.acked_transitions = acked_transitions;
        Ok(())
    }

    fn set_event_state_internal(&mut self, state: EventState) -> Result<(), Error> {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return Err(common::write_access_denied_error());
        }
        self.event_state = state.to_raw();
        Ok(())
    }

    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: u8,
        acknowledged: bool,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        if acknowledged {
            self.acked_transitions |= transition_bit & 0x07;
        } else {
            self.acked_transitions &= !(transition_bit & 0x07);
        }
        Ok(())
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::OUT_OF_SERVICE
                | PropertyIdentifier::EVENT_DETECTION_ENABLE
                | PropertyIdentifier::EVENT_ENABLE
                | PropertyIdentifier::NOTIFICATION_CLASS
        )
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests;
