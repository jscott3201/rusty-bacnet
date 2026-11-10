//! EventEnrollment (type 9) object per ASHRAE 135-2020 Clause 12.12.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, FaultParameters,
};
#[cfg(test)]
use bacnet_types::enums::ErrorCode;
use bacnet_types::enums::{
    EventState, EventType, NotifyType, ObjectType, PropertyIdentifier, Reliability,
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

/// Implement `KeepsReporting` (`write_config::reporting`) for an Event or
/// Alert Enrollment, which holds the kept rows in its own
/// fields and a `ReportingStore` in `reporting_store` (#2331).
///
/// `parameters: lend, restore, after` names the object's own side of the
/// event algorithm parameters its type keeps: the method that lends them,
/// the one that takes saved ones back, and the function a staged write of
/// one is checked by. An Alert Enrollment keeps none.
macro_rules! keep_enrollment_reporting {
    (
        $object:ty
        $(, takes_notify_type: $takes:path)?
        $(, parameters: $lend:ident, $restore:ident, $after:path)?
    ) => {
        #[cfg(feature = "event-reporting")]
        impl $crate::write_config::reporting::KeepsReporting for $object {
            fn reporting_store(&mut self) -> &mut $crate::write_config::reporting::ReportingStore {
                &mut self.reporting_store
            }

            fn reporting_store_ref(&self) -> &$crate::write_config::reporting::ReportingStore {
                &self.reporting_store
            }

            fn reporting_rows(&self) -> $crate::write_config::reporting::ReportingRows {
                #[allow(unused_mut)]
                let mut rows = $crate::write_config::reporting::ReportingRows {
                    notification_class: self.notification_class,
                    event_enable: self.event_enable,
                    notify_type: self.notify_type,
                    event_detection_enable: self.event_detection_enable,
                    options: self.event_history.options.clone(),
                    description: self.description.clone(),
                    algorithm: Default::default(),
                };
                $(rows.algorithm = self.$lend();)?
                rows
            }

            fn restore_reporting_rows(
                &mut self,
                saved: &$crate::write_config::ReportingWriteConfig,
            ) {
                $crate::write_config::reporting::RowFields {
                    notification_class: &mut self.notification_class,
                    event_enable: &mut self.event_enable,
                    notify_type: &mut self.notify_type,
                    event_detection_enable: &mut self.event_detection_enable,
                    options: &mut self.event_history.options,
                    description: &mut self.description,
                }
                .restore(saved);
                $(self.$restore(&saved.algorithm);)?
            }

            $(
                fn parameter_after(
                    &self,
                    rows: &mut $crate::write_config::AlgorithmWriteConfig,
                    write: &$crate::durable::PendingWrite,
                ) -> Result<(), bacnet_types::error::Error> {
                    $after(rows, write)
                }
            )?

            $(
                fn takes_notify_type(&self, notify_type: bacnet_types::enums::NotifyType) -> bool {
                    $takes(notify_type)
                }
            )?
        }
    };
}

mod alert;
mod creation_only;
#[cfg(feature = "event-reporting")]
mod kept;
mod metadata;
pub(crate) mod parameters;
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
    /// The reporting settings a write set, kept in storage once the
    /// enrollment opts in (#2331).
    reporting_store: crate::write_config::reporting::ReportingStore,
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
            reporting_store: Default::default(),
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

    /// Set the description string. On an enrollment keeping its reporting
    /// settings, a Description a write saved wins (#2364).
    pub fn set_description(&mut self, desc: impl Into<String>) {
        if !self.kept_over_setter(PropertyIdentifier::DESCRIPTION) {
            self.description = desc.into();
        }
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

    /// Set the structured event parameters. On an enrollment keeping its
    /// reporting settings, Event_Parameters a write saved win until
    /// `clear_written_reporting` (#2331), as for the next setter and
    /// [`set_time_delay_normal`](Self::set_time_delay_normal).
    pub fn set_event_parameters(&mut self, params: BACnetEventParameter) {
        if !self.kept_over_setter(PropertyIdentifier::EVENT_PARAMETERS) {
            self.install_event_parameters(params);
        }
    }

    /// Set the fault parameters for this event enrollment.
    pub fn set_fault_parameters(&mut self, fp: Option<FaultParameters>) {
        if !self.kept_over_setter(PropertyIdentifier::FAULT_PARAMETERS) {
            self.fault_parameters = fp;
        }
    }

    /// Take `params` as Event_Parameters, as a write, a setter and a
    /// restore do: a delay counting under the old parameters is dropped.
    fn install_event_parameters(&mut self, params: BACnetEventParameter) {
        self.event_parameters = params;
        self.pending = None;
    }

    /// Take `parameters` as a write of Fault_Parameters and a restore do.
    /// The `none` choice is the unset form (#1417).
    fn install_fault_parameters(&mut self, parameters: FaultParameters) {
        self.fault_parameters =
            (!matches!(parameters, FaultParameters::FaultNone)).then_some(parameters);
    }

    /// Take `delay` as Time_Delay_Normal, as a write, a setter and a
    /// restore do: a delay counting under the old value is dropped.
    fn install_time_delay_normal(&mut self, delay: Option<u32>) {
        self.time_delay_normal = delay;
        self.pending = None;
    }

    /// Set the event state.
    ///
    /// A configuration/seeding helper, not a lifecycle path — the evaluator
    /// commits through
    /// [`EventReporting::commit_event_transition`](crate::traits::EventReporting::commit_event_transition).
    /// It honors the same Clause 13.2.2.1 rule: while `Event_Detection_Enable` is FALSE the object
    /// must read NORMAL, so a non-NORMAL seed is ignored rather than silently
    /// breaking the invariant. Without this the public API would offer a way
    /// around a guard the rest of the object enforces.
    pub fn set_event_state(&mut self, state: EventState) {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return;
        }
        self.event_state = state;
    }

    /// Set the notification class. On an enrollment keeping its reporting
    /// settings, a value a write saved wins until `clear_written_reporting`
    /// (#2331), as for `Event_Enable`.
    pub fn set_notification_class(&mut self, nc: u32) {
        if !self.kept_over_setter(PropertyIdentifier::NOTIFICATION_CLASS) {
            self.notification_class = nc;
        }
    }

    /// Set `Event_Enable`: the transitions whose notifications are distributed.
    ///
    /// Flags outside the three named transitions are dropped.
    pub fn set_event_enable(&mut self, enable: EventTransitionBits) {
        if !self.kept_over_setter(PropertyIdentifier::EVENT_ENABLE) {
            self.event_enable = enable & EventTransitionBits::all();
        }
    }

    fn kept_over_setter(&self, property: PropertyIdentifier) -> bool {
        crate::write_config::reporting::setter_gives_way(&self.reporting_store, self.oid, property)
    }

    /// Set `Time_Delay_Normal` (the pTimeDelayNormal parameter). `None`
    /// restores the not-configured case, which takes on the
    /// `Event_Parameters` `Time_Delay` value (Clause 13.3 fallback).
    pub fn set_time_delay_normal(&mut self, delay: Option<u32>) {
        if !self.kept_over_setter(PropertyIdentifier::TIME_DELAY_NORMAL) {
            self.install_time_delay_normal(delay);
        }
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

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
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
        // through to WRITE_ACCESS_DENIED below. The evaluator commits it
        // through `EventReporting::commit_event_transition` instead, so the
        // network route and the internal lifecycle path no longer share an
        // access path (issue #130).
        if property == PropertyIdentifier::EVENT_PARAMETERS {
            self.install_event_parameters(parameters::decode_event_parameters(value)?);
            return Ok(());
        }
        if property == PropertyIdentifier::FAULT_PARAMETERS {
            self.install_fault_parameters(parameters::decode_fault_parameters(value)?);
            return Ok(());
        }
        if property == PropertyIdentifier::TIME_DELAY_NORMAL {
            // Table 12-14 codes the property O, not W; accepting the write is
            // the Clause 12.1.2 implementor's option the intrinsic types
            // already exercise, and is what makes the Clause 13.3 delay
            // asymmetry commissionable on an enrollment at all.
            let delay = parameters::written_time_delay_normal(&value)?;
            self.install_time_delay_normal(Some(delay));
            return Ok(());
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
        ))
    }

    crate::traits::lend_enrollment_eval!();

    crate::traits::lend_event_reporting!();

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.durable_capability()
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(metadata::EVENT_ENROLLMENT_PROPERTIES)
    }
}

crate::write_config::reporting::impl_reporting_persistence!(EventEnrollmentObject, without_tags);
keep_enrollment_reporting!(
    EventEnrollmentObject,
    parameters: lent_parameters, restore_parameters, kept::parameter_after
);

impl crate::creation::Creatable for EventEnrollmentObject {
    const CREATION_ONLY_PROPERTIES: &'static [PropertyIdentifier] = creation_only::CREATION_ONLY;

    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        creation_only::initialize(self, property, value)
    }
}

/// The evaluator's private state lives on the object, and Reliability
/// commits with its transition atomically.
impl crate::traits::EnrollmentEval for EventEnrollmentObject {
    /// Snapshot the pending countdown and algorithm baselines for the server
    /// evaluator.
    fn eval_state(&self) -> EventEnrollmentEvalState {
        EventEnrollmentEvalState {
            pending: self.pending.clone(),
            cov_baseline: self.cov_baseline.clone(),
            last_offnormal_value: self.last_offnormal_value,
        }
    }

    /// Store the enrollment evaluation state. Refused while
    /// `Event_Detection_Enable` is FALSE: Clause 13.2.2.1 freezes the state
    /// machine by suspending evaluation, and the reset in the
    /// write arm has already returned these fields to their initial
    /// condition, so a write arriving while disabled can only be stale.
    fn set_eval_state(&mut self, state: EventEnrollmentEvalState) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        self.pending = state.pending;
        self.cov_baseline = state.cov_baseline;
        self.last_offnormal_value = state.last_offnormal_value;
        Ok(())
    }

    fn eval_source(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        Some(self.monitored_reference)
    }

    fn set_eval_source(
        &mut self,
        source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        self.monitored_reference = source;
        Ok(())
    }

    transition::impl_event_enrollment_reliability_commit!();
}

impl crate::traits::EventReporting for EventEnrollmentObject {
    fn enrollment_summary_capability(&self) -> Option<EnrollmentSummaryCapability> {
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

    /// Refuses every commit while `Event_Detection_Enable` is FALSE
    /// (Clause 13.2.2.1), whoever calls it.
    fn commit_event_transition(
        &mut self,
        commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        transition::commit_event_transition(
            self.event_detection_enable,
            &mut self.event_state,
            &mut self.acked_transitions,
            &mut self.event_history,
            commit,
        )
    }

    /// The AcknowledgeAlarm service route (Clause 13.9). A detection-disabled
    /// enrollment, or one with no monitored reference, refuses with
    /// OBJECT/NO_ALARM_CONFIGURED, which Table 13-10 uses for an existing
    /// object lacking event-generation support or configuration: it can
    /// generate nothing, and Clause 12.12 keeps its `Acked_Transitions` at
    /// the initial condition, which an accepted ack would break.
    /// Out_Of_Service does not gate the ack: no clause forbids acknowledging
    /// a notification sent during an Out_Of_Service period.
    fn acknowledge_alarm(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<crate::event::EventStateChange>, Error> {
        if !self.event_detection_enable || self.enrollment_summary_capability().is_none() {
            return Err(crate::traits::no_alarm_configured());
        }
        self.event_history.acknowledge_correlated(
            &mut self.acked_transitions,
            event_state,
            timestamp,
        )
    }

    fn event_algorithm_inhibit_reference(
        &self,
    ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
        self.event_history.options.inhibit_reference().cloned()
    }

    fn follow_event_algorithm_inhibit(&mut self, inhibit: bool) -> bool {
        self.event_history.options.follow(inhibit)
    }
}

#[cfg(test)]
mod tests;
