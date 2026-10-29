//! EventEnrollment (type 9) object per ASHRAE 135-2020 Clause 12.12.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, FaultParameters,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, EventType, NotifyType, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::event::history::EventHistory;
use crate::event::{
    EnrollmentSummaryCapability, EventTransitionCommit, EventTransitionCommitError,
};
use crate::property_metadata::PropertyMetadata;
use crate::traits::BACnetObject;

mod alert;
mod metadata;
mod parameters;
mod state;
mod transition;
pub use alert::AlertEnrollmentObject;
pub use state::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentPending};
pub use transition::EventEnrollmentReliabilityCommit;

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
    /// The algorithm the legacy octet layout of Event_Parameters runs, which
    /// names none itself; every other alternative names its own (#1868).
    legacy_event_type: EventType,
    notify_type: NotifyType,
    event_parameters: BACnetEventParameter,
    object_property_reference: Option<BACnetDeviceObjectPropertyReference>,
    event_state: EventState,
    event_enable: EventTransitionBits,
    acked_transitions: EventTransitionBits,
    event_history: EventHistory,
    event_detection_enable: bool,
    notification_class: u32,
    fault_parameters: Option<FaultParameters>,
    status_flags: StatusFlags,
    reliability: Reliability,
    /// `Time_Delay_Normal` (property 356, Table 12-14 conformance O): Clause
    /// 12.12 feeds this value to the enrollment's algorithm as its
    /// pTimeDelayNormal input. `None` is the not-configured case and takes on
    /// the `Time_Delay` carried inside `event_parameters` (Table 12-15 maps
    /// `Time_Delay` to pTimeDelay for every evaluated algorithm), following
    /// Clause 13.3's fallback from absent pTimeDelayNormal to pTimeDelay.
    time_delay_normal: Option<u32>,
    /// Delayed transition counting down, if any. In-memory only.
    pending: Option<EventEnrollmentPending>,
    /// Effective source of the private evaluation state. In-memory only.
    monitored_reference: Option<EventEnrollmentMonitoredSource>,
    /// CHANGE_OF_VALUE detection baseline (Clause 13.3.3). In-memory only.
    cov_baseline: Option<PropertyValue>,
    /// Domain-tagged monitored value that caused the last OFFNORMAL transition
    /// (Clause 13.3.2 condition (c)). In-memory only.
    last_offnormal_value: Option<u64>,
}

impl EventEnrollmentObject {
    /// Create a new EventEnrollment object.
    ///
    /// Event_Type follows Event_Parameters (Clause 12.12.5): it reads NONE
    /// while the parameters are the unset `none` alternative and the
    /// matching type once parameters are set. `legacy_event_type` only
    /// matters for the pre-framing octet layout of Event_Parameters, which
    /// carries no algorithm of its own: it is the algorithm that layout runs
    /// and the Event_Type served while it is held.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        legacy_event_type: EventType,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            legacy_event_type,
            notify_type: NotifyType::ALARM,
            event_parameters: Self::UNSET_EVENT_PARAMETERS,
            object_property_reference: None,
            event_state: EventState::NORMAL,
            event_enable: EventTransitionBits::all(),
            // Clause 12.12 starts each flag TRUE until a transition of its
            // kind first happens on the object. That all-TRUE initial value
            // is also the initial condition the detection-disabled reset
            // restores, so `RESET_ACKED_TRANSITIONS` names it once.
            acked_transitions: Self::RESET_ACKED_TRANSITIONS,
            event_history: EventHistory::default(),
            event_detection_enable: true,
            notification_class: 0,
            fault_parameters: None,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            // Absent so the delay behavior equals the normative pTimeDelay
            // fallback until a client writes the property — never an error,
            // never a zero.
            time_delay_normal: None,
            pending: None,
            monitored_reference: None,
            cov_baseline: None,
            last_offnormal_value: None,
        })
    }

    /// `Acked_Transitions` in its initial condition: every transition flag TRUE,
    /// the value a flag holds until its transition kind first occurs on the
    /// object (ASHRAE 135-2020 Clause 12.12).
    const RESET_ACKED_TRANSITIONS: EventTransitionBits = EventTransitionBits::all();

    /// `Event_Parameters` before anything sets them: the `none` alternative
    /// of the BACnetEventParameter CHOICE (context tag 20, a NULL; Clause
    /// 21), so an unset enrollment serves a member of the property's
    /// datatype (#1868). The evaluator has no algorithm to run for it.
    const UNSET_EVENT_PARAMETERS: BACnetEventParameter = BACnetEventParameter::None;

    /// Put the object in the state ASHRAE 135-2020 Clause 13.2.2.1 prescribes
    /// for disabled detection (`Event_Detection_Enable` FALSE): Event_State
    /// reads NORMAL, the three per-transition properties (Event_Time_Stamps,
    /// Event_Message_Texts, Acked_Transitions) go back to their initial
    /// values, and no transition is generated while detection stays off.
    ///
    /// The monitored-source identity, pending countdown, and both baselines
    /// are cleared too: they are extensions of the same event-state-detection
    /// state machine whose evaluation the clause suspends, so a stale
    /// countdown must not survive into the next
    /// enabled period and fire against a condition the object no longer
    /// observes. The intrinsic types make the same choice for their detectors
    /// (`analog/input.rs` clears `detector.pending` on the identical write).
    /// The COV baseline's initialization on re-enable is the local matter
    /// Clause 13.3.3 assigns it; clearing is consistent with the first-sample
    /// policy.
    fn apply_detection_disabled_reset(&mut self) {
        self.event_state = EventState::NORMAL;
        self.acked_transitions = Self::RESET_ACKED_TRANSITIONS;
        self.event_history.reset();
        self.pending = None;
        self.monitored_reference = None;
        self.cov_baseline = None;
        self.last_offnormal_value = None;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set Object_Property_Reference, the property the enrollment monitors,
    /// or `None` for none. The property is read-only over the network.
    /// A reference whose object or Device is at the reserved instance
    /// 4194303 is the unset form, so it leaves the enrollment without one
    /// (#1417).
    ///
    /// A reference whose device identifier isn't a Device object is refused
    /// with PROPERTY / VALUE_OUT_OF_RANGE and the reference set before is
    /// kept (#1308).
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetDeviceObjectPropertyReference>,
    ) -> Result<(), Error> {
        if let Some(reference) = &reference {
            crate::device_reference::check_device_member(reference.device_identifier)?;
        }
        self.object_property_reference = reference.and_then(crate::device_reference::set_or_unset);
        self.pending = None;
        Ok(())
    }

    /// Set the structured event parameters.
    pub fn set_event_parameters(&mut self, params: BACnetEventParameter) {
        self.event_parameters = params;
        self.pending = None;
    }

    /// Set the fault parameters for this event enrollment.
    pub fn set_fault_parameters(&mut self, fp: Option<FaultParameters>) {
        self.fault_parameters = fp;
    }

    /// Set the event state.
    ///
    /// A configuration/seeding helper, not a lifecycle path — the evaluator
    /// uses [`BACnetObject::set_event_state_internal`]. It honors the same
    /// Clause 13.2.2.1 rule: while `Event_Detection_Enable` is FALSE the object
    /// must read NORMAL, so a non-NORMAL seed is ignored rather than silently
    /// breaking the invariant. Without this the public API would offer a way
    /// around a guard the rest of the object enforces.
    pub fn set_event_state(&mut self, state: EventState) {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return;
        }
        self.event_state = state;
    }

    /// Set the notification class.
    pub fn set_notification_class(&mut self, nc: u32) {
        self.notification_class = nc;
    }

    /// Set `Event_Enable`: the transitions whose notifications are distributed.
    ///
    /// Flags outside the three named transitions are dropped.
    pub fn set_event_enable(&mut self, enable: EventTransitionBits) {
        self.event_enable = enable & EventTransitionBits::all();
    }

    /// Set `Time_Delay_Normal` (the pTimeDelayNormal parameter). `None`
    /// restores the not-configured case, which takes on the
    /// `Event_Parameters` `Time_Delay` value (Clause 13.3 fallback).
    pub fn set_time_delay_normal(&mut self, delay: Option<u32>) {
        self.time_delay_normal = delay;
        self.pending = None;
    }

    /// The pTimeDelay the stored `Event_Parameters` supply: the `time_delay`
    /// member of every algorithm that has one (Table 12-15), so an unset
    /// Time_Delay_Normal reads back the algorithm's delay (#2090), whether
    /// or not the server runs it. The alternatives with no time-delay slot
    /// (`none`, buffer-ready, access-event, extended, an unassigned tag and
    /// the `0xFF` legacy octet layout) contribute zero, so their evaluation
    /// fires immediately, exactly as it did before delay honoring existed.
    fn event_parameters_time_delay(&self) -> u32 {
        use BACnetEventParameter as P;
        match &self.event_parameters {
            P::ChangeOfBitstring { time_delay, .. }
            | P::ChangeOfState { time_delay, .. }
            | P::ChangeOfValue { time_delay, .. }
            | P::FloatingLimit { time_delay, .. }
            | P::OutOfRange { time_delay, .. }
            | P::CommandFailure { time_delay, .. }
            | P::ChangeOfLifeSafety { time_delay, .. }
            | P::UnsignedRange { time_delay, .. }
            | P::DoubleOutOfRange { time_delay, .. }
            | P::SignedOutOfRange { time_delay, .. }
            | P::UnsignedOutOfRange { time_delay, .. }
            | P::ChangeOfCharacterstring { time_delay, .. }
            | P::ChangeOfStatusFlags { time_delay, .. }
            | P::ChangeOfDiscreteValue { time_delay }
            | P::ChangeOfTimer { time_delay, .. } => *time_delay,
            _ => 0,
        }
    }

    /// Event_Type as Event_Parameters gives it (Clause 12.12.5): each
    /// framed alternative's tag is its event type (`none`, tag 20, is NONE).
    /// The legacy octet layout uses the constructor's type, and so does an
    /// opaque alternative at tag 64 or above: such a tag names no event type
    /// the standard reserves (0 to 63), so nothing is derived from it.
    fn event_type(&self) -> EventType {
        match &self.event_parameters {
            BACnetEventParameter::Opaque { tag, .. } if *tag >= 64 => self.legacy_event_type,
            parameters => EventType::from_raw(u32::from(parameters.tag())),
        }
    }

    fn effective_fault_parameters(&self) -> &FaultParameters {
        self.fault_parameters
            .as_ref()
            .unwrap_or(&FaultParameters::FaultNone)
    }
}

impl BACnetObject for EventEnrollmentObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        // Any enrollment with a monitored reference can fault: the monitored
        // object's fault and the configuration checks run before the
        // parameters are looked at, so whatever Event_Parameters hold (`none`,
        // an unmodeled alternative, Extended) its TO_FAULT is listed and
        // acknowledged (#1868). Event_Detection_Enable FALSE is left to the
        // summary and acknowledgment paths.
        self.object_property_reference
            .is_some()
            .then(|| EnrollmentSummaryCapability {
                event_type: self.event_type(),
                last_transition: self.event_history.last_transition(),
            })
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::STATUS_FLAGS {
            // Clause 12.12 fixes OVERRIDDEN and OUT_OF_SERVICE at FALSE;
            // IN_ALARM follows Event_State and FAULT follows Reliability.
            return Ok(common::compute_status_flags(
                StatusFlags::empty(),
                self.reliability,
                false,
                self.event_state,
            ));
        }
        // Table 12-14 has no Out_Of_Service (#1064).
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        // Event_Time_Stamps, Event_Message_Texts and the reporting options
        // (Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair,
        // #1507), all optional or required rows of Table 12-14.
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::EVENT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_TYPE => {
                Ok(PropertyValue::Enumerated(self.event_type().to_raw()))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.notify_type.to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_PARAMETERS => {
                let mut buf = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_event_parameter(
                    &mut buf,
                    &self.event_parameters,
                )?;
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            // Table 12-14's BACnetDeviceObjectPropertyReference, its optional
            // index and Device members present only when set (#1182); the
            // unset form while the enrollment has no reference (#1417).
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                Ok(crate::device_reference::optional_reference_value(
                    self.object_property_reference.as_ref(),
                    ObjectType::ANALOG_INPUT,
                ))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_enable.to_bacnet()],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.acked_transitions.to_bacnet()],
            }),
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::FAULT_TYPE => Ok(PropertyValue::Enumerated(
                self.effective_fault_parameters().fault_type().to_raw(),
            )),
            p if p == PropertyIdentifier::FAULT_PARAMETERS => {
                let mut buf = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_fault_parameters(
                    &mut buf,
                    self.effective_fault_parameters(),
                )?;
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            p if p == PropertyIdentifier::TIME_DELAY_NORMAL => {
                // Clause 13.3 supplies pTimeDelay as the fallback when
                // pTimeDelayNormal is absent —
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
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::NOTIFY_TYPE {
            // BACnetNotifyType is a closed three-value production {alarm,
            // event, ack-notification} (Clause 21); out-of-production values
            // are PROPERTY / VALUE_OUT_OF_RANGE (Clause 15.9.1.3).
            if let PropertyValue::Enumerated(v) = value {
                let notify_type = NotifyType::from_raw(v);
                if !NotifyType::ALL_NAMED.iter().any(|&(_, n)| n == notify_type) {
                    return Err(common::value_out_of_range_error());
                }
                self.notify_type = notify_type;
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
                self.event_enable = EventTransitionBits::from_bacnet(&[byte]);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                // Clause 12.12 states the disabled condition as an invariant —
                // disabled detection requires a persistent NORMAL Event_State —
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
            self.set_event_parameters(parameters::decode_event_parameters(value)?);
            return Ok(());
        }
        if property == PropertyIdentifier::FAULT_PARAMETERS {
            // The context-tagged `none` choice is the unset form (#1417).
            let parameters = parameters::decode_fault_parameters(value)?;
            self.fault_parameters =
                (!matches!(parameters, FaultParameters::FaultNone)).then_some(parameters);
            return Ok(());
        }
        if property == PropertyIdentifier::TIME_DELAY_NORMAL {
            // Table 12-14 codes the property O, not W; accepting the write is
            // the Clause 12.1.2 implementor's option the intrinsic types
            // already exercise, and is what makes the Clause 13.3 delay
            // asymmetry commissionable on an enrollment at all.
            if let PropertyValue::Unsigned(v) = value {
                self.set_time_delay_normal(Some(common::u64_to_u32(v)?));
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            self.event_history
                .write(property, array_index, &value, self.event_detection_enable)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    /// Internal lifecycle path for the algorithmically-derived `Event_State`.
    ///
    /// The evaluator calls this — not `write_property(EVENT_STATE, …)` — so the
    /// network route (which rejects `EVENT_STATE`) and the internal lifecycle
    /// path are distinct (issue #130). Stores the modeled state verbatim; the
    /// only caller is the trusted server evaluator.
    ///
    /// Refuses any non-NORMAL state while `Event_Detection_Enable` is FALSE.
    /// Clause 13.2.2.1 prohibits transitions in that case,
    /// and the server evaluator already skips such objects — this guard makes
    /// the invariant hold by construction rather than by the caller
    /// remembering, so a future caller cannot reintroduce the violation.
    fn set_event_state_internal(&mut self, state: EventState) -> Result<(), Error> {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return Err(common::write_access_denied_error());
        }
        self.event_state = state;
        Ok(())
    }

    /// Snapshot the pending countdown and algorithm baselines for the server
    /// evaluator.
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        Some(EventEnrollmentEvalState {
            pending: self.pending.clone(),
            cov_baseline: self.cov_baseline.clone(),
            last_offnormal_value: self.last_offnormal_value,
        })
    }

    /// Store the enrollment evaluation state. Refused while
    /// `Event_Detection_Enable` is FALSE: Clause 13.2.2.1 freezes the state
    /// machine by suspending evaluation, and the reset in the
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

    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        Some(self.monitored_reference)
    }

    fn set_enrollment_eval_source_internal(
        &mut self,
        source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        self.monitored_reference = source;
        Ok(())
    }

    /// Acknowledge an alarm transition (the AcknowledgeAlarm service route,
    /// Clause 13.9): Clause 13.2.3 sets the bit on the acknowledgment
    /// indication — unconditional and idempotent, so a repeated ack succeeds
    /// again. A detection-DISABLED enrollment instead refuses with
    /// OBJECT/NO_ALARM_CONFIGURED, which Table 13-10 uses for an existing object
    /// lacking event-generation support or configuration: it can
    /// generate nothing, and Clause 12.12 keeps its `Acked_Transitions` at
    /// the initial condition, which an accepted ack would break.
    /// Out_Of_Service does not gate the ack: no clause bars acknowledging a
    /// notification already issued while the object is out of service.
    fn acknowledge_alarm(
        &mut self,
        transition_bit: EventTransitionBits,
    ) -> Result<(), bacnet_types::error::Error> {
        if !self.event_detection_enable {
            return Err(bacnet_types::error::Error::protocol(
                bacnet_types::enums::ErrorClass::OBJECT,
                bacnet_types::enums::ErrorCode::NO_ALARM_CONFIGURED,
            ));
        }
        self.acked_transitions |= transition_bit & EventTransitionBits::all();
        Ok(())
    }

    fn acknowledge_alarm_correlated_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<(), Error> {
        if !self.event_detection_enable || self.enrollment_summary_capability_internal().is_none() {
            return Err(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::NO_ALARM_CONFIGURED,
            ));
        }
        self.event_history.acknowledge_correlated(
            &mut self.acked_transitions,
            event_state,
            timestamp,
        )
    }

    fn acknowledge_alarm_correlated_detailed_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<crate::event::EventStateChange>, Error> {
        if !self.event_detection_enable || self.enrollment_summary_capability_internal().is_none() {
            return Err(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::NO_ALARM_CONFIGURED,
            ));
        }
        self.event_history.acknowledge_correlated_detailed(
            &mut self.acked_transitions,
            event_state,
            timestamp,
        )
    }

    /// Clause 13.2.3's transition-received maintenance of `Acked_Transitions`:
    /// the evaluator resolves `Ack_Required` from the referenced Notification
    /// Class object and this call applies the outcome — clear the bit when
    /// ack is required, set it otherwise. Refused while detection is
    /// disabled, preserving the same invariant as above: Acked_Transitions
    /// must retain its initial value throughout the disabled period.
    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: EventTransitionBits,
        acknowledged: bool,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        self.acked_transitions
            .set(transition_bit & EventTransitionBits::all(), acknowledged);
        Ok(())
    }

    transition::impl_event_enrollment_transition_commit!();

    /// Mirrors the `write_property` arms above, so PICS reports what dispatch
    /// actually accepts.
    ///
    /// Enumerated explicitly: the intrinsic-reporting objects accept
    /// `HIGH_LIMIT`, `LOW_LIMIT`, `DEADBAND`, `LIMIT_ENABLE` and `TIME_DELAY`,
    /// none of which an Event Enrollment accepts, because it carries those
    /// inside `Event_Parameters` instead. `TIME_DELAY_NORMAL` is accepted here
    /// as a real (O-coded) property, per Table 12-14.
    ///
    /// `Event_Detection_Enable` is writable even though Table 12-14 codes it R
    /// rather than W: Clause 12.1.2 lets implementors accept writes to an R
    /// property unless that object's property description expressly forbids
    /// them. Clause 12.12 has no such prohibition; it anticipates setting the
    /// value during configuration without mandating that timing. Annex K's
    /// AE-AVM-A BIBB (Table K-17) positively requires a conforming workstation
    /// to be able to *write* this property, so refusing the write would be
    /// interoperably hostile.
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Table 12-14 has no Out_Of_Service (#1064), so of the common
        // writable rows only the name and description apply.
        matches!(
            property,
            PropertyIdentifier::OBJECT_NAME
                | PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::NOTIFY_TYPE
                | PropertyIdentifier::NOTIFICATION_CLASS
                | PropertyIdentifier::EVENT_ENABLE
                | PropertyIdentifier::EVENT_DETECTION_ENABLE
                | PropertyIdentifier::EVENT_PARAMETERS
                | PropertyIdentifier::FAULT_PARAMETERS
                | PropertyIdentifier::TIME_DELAY_NORMAL
                | PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG
                | PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF
                | PropertyIdentifier::EVENT_ALGORITHM_INHIBIT
        )
    }

    fn event_algorithm_inhibit_reference_internal(
        &self,
    ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
        self.event_history.options.inhibit_reference().cloned()
    }

    fn follow_event_algorithm_inhibit_internal(&mut self, inhibit: bool) -> bool {
        self.event_history.options.follow(inhibit)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(metadata::EVENT_ENROLLMENT_PROPERTIES)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(metadata::EVENT_ENROLLMENT_PROPERTIES)
    }
}

#[cfg(test)]
mod tests;
