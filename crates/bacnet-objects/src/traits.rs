//! BACnetObject trait — the interface all BACnet objects implement.

use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use bacnet_types::constructed::BACnetLogRecord;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, LifeSafetyOperation, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::clock::ClockReader;
use crate::event::TransitionOutcome;
use crate::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use crate::file::FileStorage;

/// Object-owned state that cannot be reconstructed from property readback.
///
/// This token supports the server's stronger-than-Clause-15.10 rollback policy
/// for WritePropertyMultiple. The server still snapshots readable property
/// values when a token exists; the token supplements that snapshot with state
/// hidden by readback. This includes event-detection resets, fallback-backed
/// values, destructive log writes, and writes that update derived properties.
#[doc(hidden)]
pub struct WritePropertyRollback(Box<dyn Any + Send + Sync>);

/// Result of applying a LifeSafetyOperation to an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSafetyOperationEffect {
    /// The operation changed object state.
    Applied,
    /// The requested idempotent state was already present.
    AlreadyApplied,
}

impl WritePropertyRollback {
    /// Wrap object-private rollback state.
    #[doc(hidden)]
    pub fn new<T: Any + Send + Sync>(state: T) -> Self {
        Self(Box::new(state))
    }

    /// Recover object-private rollback state.
    #[doc(hidden)]
    pub fn downcast<T: Any + Send + Sync>(self) -> Result<T, Error> {
        self.0.downcast::<T>().map(|state| *state).map_err(|_| {
            Error::Encoding("object received an incompatible write rollback token".into())
        })
    }
}

/// The core trait for all BACnet objects.
///
/// Implementors represent a single BACnet object (Device, AnalogInput, etc.)
/// and provide read/write access to their properties.
pub trait BACnetObject: Send + Sync {
    /// The object's identifier (type + instance).
    fn object_identifier(&self) -> ObjectIdentifier;

    /// The object's name.
    fn object_name(&self) -> &str;

    /// Read a property value.
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error>;

    /// Write a property value.
    ///
    /// Returning `Err` MUST leave the object unchanged. WritePropertyMultiple
    /// can restore earlier successful writes, but it cannot reconstruct a
    /// write-only property that mutates before rejecting its own write.
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error>;

    /// Return canonical metadata for this object's effective property rows.
    ///
    /// Migrated implementations return every supported standard row for the
    /// current instance, including `PROPERTY_LIST`. Borrowed rows cover static
    /// or object-owned metadata; owned rows support dynamically assembled
    /// per-instance sets. An empty borrowed default marks an object as unmigrated.
    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        Cow::Borrowed(&[])
    }

    /// List all properties this object supports in the legacy projection.
    ///
    /// For migrated objects this includes Object_Identifier, Object_Name, and
    /// Object_Type but omits Property_List. Reading the BACnet Property_List
    /// property applies the additional wire-level universal-property filter.
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]>;

    /// Bind or remove the database's shared wall-clock reader.
    ///
    /// Objects that do not expose clock-derived state ignore this internal
    /// lifecycle hook.
    #[doc(hidden)]
    fn bind_clock_internal(&mut self, _clock: Option<Arc<dyn ClockReader>>) {}

    /// Whether `write_property` accepts `property` for this object.
    ///
    /// PICS generation and runtime dispatch MUST consult this (or
    /// `write_property` itself) rather than a separate heuristic, so the PICS
    /// writable flags cannot drift from the actual write routes. The default
    /// reproduces the historical PICS heuristic (see
    /// [`historical_writable_default`]) so unmigrated object types keep their
    /// current PICS output. Object implementations override to mirror their
    /// real `write_property` arms exactly.
    ///
    /// Universal read-only properties (`OBJECT_IDENTIFIER`, `OBJECT_TYPE`,
    /// `PROPERTY_LIST`, `STATUS_FLAGS`) are always non-writable and are
    /// excluded by the default; overrides should preserve that invariant.
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        let metadata = self.property_metadata();
        if metadata.is_empty() {
            historical_writable_default(self.object_identifier().object_type(), property)
        } else {
            crate::property_metadata::is_writable_in_metadata(metadata.as_ref(), property)
        }
    }

    /// Whether `property` accepts an array index on this object.
    ///
    /// Per Clause 12.1.5.1, only BACnetARRAY (and BACnetARRAY of BACnetLIST)
    /// properties accept an array index; Clause 12.1.5.2 makes ReadRange the
    /// only positional access to a BACnetLIST. The RP/RPM/WP/WPM service
    /// handlers gate the request's array index on this query and reject a
    /// supplied index on a non-array property with PROPERTY /
    /// PROPERTY_IS_NOT_AN_ARRAY (Clause 15.5.1.3, Clause 15.9.1.3).
    ///
    /// The default reproduces the standard's classification (see
    /// [`array_property_default`]): identifier-stable arrays are admitted
    /// without consulting the object type, the identifiers whose datatype
    /// changes with the object type (ALARM_VALUES / FAULT_VALUES,
    /// LIST_OF_OBJECT_PROPERTY_REFERENCES, PRESENT_VALUE) classify by
    /// `object_identifier().object_type()`, and everything else — scalars and
    /// BACnetLIST properties — rejects the index. Object implementations with
    /// vendor or per-instance array properties override.
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        array_property_default(self.object_identifier().object_type(), property)
    }

    /// Capture state that property readback cannot preserve during rollback.
    ///
    /// The default returns `None`; the server snapshots readable property
    /// values before calling this hook. Implementations may return a token for
    /// writes whose side effects, destructive behavior, or fallback-backed
    /// storage make replay lossy. A destructive write may move state into the
    /// token when `value` is valid; the token is restored if the write or a
    /// later write fails, and dropped when the WPM request succeeds. Returning
    /// `None` MUST leave the object unchanged.
    #[doc(hidden)]
    fn capture_write_property_rollback(
        &mut self,
        _property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        None
    }

    /// Restore a token returned by [`capture_write_property_rollback`](Self::capture_write_property_rollback).
    #[doc(hidden)]
    fn restore_write_property_rollback(
        &mut self,
        _rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        Err(Error::Encoding(
            "object does not support this write rollback token".into(),
        ))
    }

    /// Whether this object type can be created at runtime via CreateObject.
    ///
    /// Default `false`; override `true` only for types the network factory
    /// (`handle_create_object`) actually constructs, so PICS createability
    /// matches the runtime factory with no separate list to drift.
    fn is_createable(&self) -> bool {
        false
    }

    /// Whether this object type can be deleted at runtime via DeleteObject.
    ///
    /// Default `true`; override `false` on object types that are not
    /// deleteable (e.g. `Device`, `NetworkPort`).
    fn is_deleteable(&self) -> bool {
        true
    }

    /// List the REQUIRED properties for this object type.
    ///
    /// Migrated objects derive this set from canonical `R` and `W` rows,
    /// including Property_List. Unmigrated objects retain the historical four
    /// universal properties. Service-specific consumers may exclude
    /// Property_List where their protocol contract requires it.
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        let metadata = self.property_metadata();
        if !metadata.is_empty() {
            return crate::property_metadata::required_properties_from_metadata(metadata.as_ref());
        }
        static UNIVERSAL: [PropertyIdentifier; 4] = [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
        ];
        Cow::Borrowed(&UNIVERSAL)
    }

    /// Whether this object type supports COV notifications.
    ///
    /// Override to return `true` for object types that can generate COV
    /// notifications (analog, binary, multi-state I/O/V). Default is `false`.
    fn supports_cov(&self) -> bool {
        false
    }

    /// COV increment for this object (analog objects only).
    ///
    /// Returns `Some(increment)` for objects that use COV_Increment filtering
    /// (e.g., AnalogInput, AnalogOutput, AnalogValue). A notification fires only
    /// when `|current_value - last_notified_value| >= increment`.
    ///
    /// Returns `None` for objects that notify on any state change (binary, multi-state).
    fn cov_increment(&self) -> Option<f32> {
        None
    }

    /// Set the OVERRIDDEN bit in StatusFlags.
    ///
    /// For software-only objects this is always FALSE per spec. Hardware
    /// integrations can override to set TRUE when present_value is overridden
    /// by physical means (e.g., a manual switch on an output).
    fn set_overridden(&mut self, _overridden: bool) {}

    /// Evaluate intrinsic reporting after a present_value change.
    ///
    /// This is the per-write entry point: it seeds (or cancels) a pending
    /// delayed transition and fires immediately only when `Time_Delay == 0`.
    /// It never advances the `Time_Delay` countdown — repeated writes to the
    /// same value do not shorten the delay (per ASHRAE 135-2020 §13.2.4 the
    /// countdown advances once per elapsed second via
    /// [`tick_intrinsic_reporting`](Self::tick_intrinsic_reporting)).
    ///
    /// Returns `Some(TransitionOutcome)` whenever a transition fired, or
    /// `None` when none did (no change, delay seeded, or the object does not
    /// support intrinsic reporting). A cleared `Event_Enable` bit sets the
    /// outcome's `distribute` flag to false rather than withholding the
    /// transition — Clause 13.2.2.1.4's transition actions run either way, and
    /// `Event_Enable` disables only external distribution, downstream in the
    /// notification-distribution process (Clause 13.2.5).
    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        None
    }

    /// Advance the `Time_Delay` countdown for a pending transition.
    ///
    /// Called by the server's one-second intrinsic-reporting task. Fires the
    /// pending transition when its delay elapses this tick, cancels it if the
    /// triggering condition reverted, and returns `Some(TransitionOutcome)`
    /// when a transition fires. As with
    /// [`evaluate_intrinsic_reporting`](Self::evaluate_intrinsic_reporting),
    /// `Event_Enable` is reported via `distribute`, not by returning `None`.
    /// Objects without a delayed transition return `None`.
    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        None
    }

    /// Evaluate this object's schedule for the given time.
    ///
    /// Returns `Some((new_value, refs))` if the present value changed, where `refs`
    /// is the list of (object_identifier, property_identifier) pairs to write to.
    /// Only meaningful for Schedule objects; default returns `None`.
    fn tick_schedule(
        &mut self,
        _day_of_week: u8,
        _hour: u8,
        _minute: u8,
    ) -> Option<(PropertyValue, Vec<(ObjectIdentifier, u32)>)> {
        None
    }

    /// Acknowledge an alarm transition. Sets the corresponding bit in acked_transitions.
    /// Returns Ok(()) if the object supports event detection, Err otherwise.
    fn acknowledge_alarm(&mut self, _transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        Err(bacnet_types::error::Error::Protocol {
            class: bacnet_types::enums::ErrorClass::OBJECT.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw()
                as u32,
        })
    }

    /// Apply a LifeSafetyOperation atomically to this object.
    ///
    /// Implementations must leave the object unchanged when returning `Err`.
    /// They run synchronously under the object-database write lock and must be
    /// fast, nonblocking, and panic-free. External or irreversible actuation
    /// also needs an application-owned idempotency/replay contract. The default
    /// reports that the object does not support this service.
    fn apply_life_safety_operation(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Set the next LifeSafetyOperation expected by trusted local logic.
    ///
    /// This is an application-facing state channel, not a network property
    /// write. The default reports that the object does not support the state.
    fn set_life_safety_operation_expected_internal(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply an internally-detected `Event_State` transition.
    ///
    /// This is the **internal** lifecycle path for the algorithmically-derived
    /// `Event_State` on objects such as Event Enrollment (ASHRAE 135-2020
    /// Clause 12.12). It is deliberately distinct from the network
    /// [`write_property`](Self::write_property) route: `Event_State` is
    /// read-only over the network, so network writes are rejected while the
    /// server's evaluator reaches the field through this method. Objects
    /// without an algorithmic `Event_State` return
    /// `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`.
    ///
    /// The **default** returns `Err`, so objects without an algorithmic
    /// `Event_State` opt out. Objects that do model one (e.g.
    /// `EventEnrollmentObject`) override this to store the value verbatim:
    /// the only caller is a trusted internal evaluator that passes a modeled
    /// [`EventState`], mirroring the inherent `set_event_state` builder and
    /// the existing read arm. Network-facing validation — rejecting all
    /// `Event_State` writes — lives in [`write_property`](Self::write_property),
    /// not here. Implementations must leave `Event_State` unchanged when they
    /// return `Err`.
    fn set_event_state_internal(&mut self, _state: EventState) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Snapshot this object's Event Enrollment evaluation state, if it models one.
    ///
    /// This is the read half of the internal channel the server's Event
    /// Enrollment evaluator uses to persist per-enrollment algorithm state
    /// across evaluation cycles: the pending (delayed) transition countdown,
    /// the CHANGE_OF_VALUE detection baseline (Clause 13.3.3: "the value of
    /// the monitored value when a transition to NORMAL is indicated"), and
    /// the value that caused the last transition to OFFNORMAL (Clause 13.3.2
    /// condition (c)). Like [`set_event_state_internal`](Self::set_event_state_internal)
    /// it deliberately bypasses the network property model: none of the three
    /// slots is a BACnet property, and 135-2020 assigns their initialization
    /// to local matters.
    ///
    /// The default returns `None` — objects without algorithmic event
    /// detection carry no such state, and the evaluator treats `None` as an
    /// empty state it cannot write back (delay honoring and the COV baseline
    /// then stay unavailable, matching this crate's pre-delay behavior).
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        None
    }

    /// Store this object's Event Enrollment evaluation state.
    ///
    /// The write half of [`enrollment_eval_state_internal`](Self::enrollment_eval_state_internal).
    /// The only caller is the trusted server evaluator, passing a state it
    /// derived from a prior snapshot plus the current cycle's evaluation.
    /// Implementations enforce the Clause 13.2.2.1 invariant by construction:
    /// while `Event_Detection_Enable` is FALSE "no transitions shall occur",
    /// so a write arriving then is refused rather than queued (and the
    /// detection-disable reset has already cleared the fields).
    ///
    /// The **default** returns `Err`, so objects without enrollment evaluation
    /// state opt out and the evaluator's write-back is dropped, never stored
    /// into an object that does not model it.
    fn set_enrollment_eval_state_internal(
        &mut self,
        _state: EventEnrollmentEvalState,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Snapshot the monitored source that owns Event Enrollment private state.
    ///
    /// The outer `Option` indicates whether the object supports this channel;
    /// the inner `Option` is empty before a source has been established.
    /// The server stores source ownership in its object database when an
    /// object implements evaluation state but leaves this channel unsupported.
    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        None
    }

    /// Store or clear the monitored source that owns Event Enrollment state.
    fn set_enrollment_eval_source_internal(
        &mut self,
        _source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Set or clear one `Acked_Transitions` bit on a received event-state
    /// transition.
    ///
    /// Implements the alarm-acknowledgment half of Clause 13.2.2.1.4's fourth
    /// transition action ("indicate the transition to the Alarm-Acknowledgment
    /// process"), per Clause 13.2.3: "When an event state transition is
    /// received, the corresponding bit in Acked_Transitions is either set or
    /// cleared. If the corresponding bit in Ack_Required is set, then the bit
    /// in Acked_Transitions is cleared, otherwise it is set." The caller (the
    /// server evaluator) resolves `Ack_Required` from the referenced
    /// Notification Class object and passes the outcome as `acknowledged`;
    /// this method performs only the bit maintenance.
    ///
    /// `transition_bit` is the transition direction's bit mask in
    /// `Acked_Transitions`' internal bit0-first form (`0x01` TO_OFFNORMAL,
    /// `0x02` TO_FAULT, `0x04` TO_NORMAL). The set half overlaps the
    /// network-reachable [`acknowledge_alarm`](Self::acknowledge_alarm), which
    /// also ORs the bit in per Clause 13.2.3's acknowledgment-indication
    /// paragraph; the clear half has no network route by design (a property
    /// write could fabricate or erase acknowledgments — see the
    /// `write_generic_event_properties!` denial comment).
    ///
    /// The **default** returns `Err`, so objects without an algorithmic
    /// `Acked_Transitions` opt out.
    fn set_acked_transitions_internal(
        &mut self,
        _transition_bit: u8,
        _acknowledged: bool,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply an internally-derived `Reliability` value.
    ///
    /// This is the **internal** reliability-evaluation path, distinct from the
    /// network [`write_property`](Self::write_property) route. Implementations
    /// enforce symmetric ownership: clients may write while `Out_Of_Service`
    /// is TRUE, and internal evaluation may write while it is FALSE. ASHRAE
    /// 135-2020 Clause 3.2 defines reliability evaluation as "the process by
    /// which an object determines its reliability and thus the value to set
    /// into its Reliability property."
    ///
    /// The default rejects the operation, so object types without an internal
    /// reliability-evaluation process remain unaffected.
    fn set_reliability_internal(&mut self, _reliability: u32) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Borrow this object's File storage, if it has any.
    ///
    /// This is the read half of the **internal** channel the server's
    /// AtomicReadFile handler uses to reach file contents. Like
    /// [`set_event_state_internal`](Self::set_event_state_internal) it
    /// bypasses the property model on purpose: Table 12-16 (ASHRAE 135-2020
    /// Clause 12.13) defines no File Data property, so file contents are
    /// reachable only through the Clause 14 File Access Services.
    ///
    /// The **default** returns `None`, so object types without a file opt
    /// out; the server reports `None` on a File-typed object as SERVICES /
    /// FILE_ACCESS_DENIED (Clause 18: "a file that is currently locked or
    /// otherwise not accessible") rather than reading it as empty.
    /// Applications backing a File object with their own storage — a disk
    /// file, a firmware partition — implement [`FileStorage`] and return
    /// `Some`.
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        None
    }

    /// Mutably borrow this object's File storage, if it has any.
    ///
    /// The write half of
    /// [`file_storage_internal`](Self::file_storage_internal), used by the
    /// AtomicWriteFile handler after the read-only and access-method gates
    /// have passed. The **default** returns `None`.
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        None
    }

    /// Add a trend log record (only meaningful for TrendLog / TrendLogMultiple).
    ///
    /// Default is a no-op. TrendLog objects override to append to their buffer.
    fn add_trend_record(&mut self, _record: BACnetLogRecord) {}
}

/// The default array/list classification behind
/// [`BACnetObject::is_array_property`], keyed by the Clause 12 property
/// tables. Three identifier classes:
///
/// - **Identifier-stable BACnetARRAY** properties admit an index on every
///   object type that defines them: OBJECT_LIST (Table 12-13), PROPERTY_LIST
///   (every table), STATE_TEXT (Tables 12-21/12-22/12-23), PRIORITY
///   (Table 12-24), WEEKLY_SCHEDULE / EXCEPTION_SCHEDULE (Table 12-28),
///   EVENT_TIME_STAMPS / EVENT_MESSAGE_TEXTS (Table 12-2 family),
///   PRIORITY_ARRAY (the commandable family), TAGS (Annex Y),
///   SUBORDINATE_LIST / SUBORDINATE_ANNOTATIONS (Table 12-34),
///   GROUP_MEMBERS / GROUP_MEMBER_NAMES (Table 12-57; Elevator/Lift also type
///   GROUP_MEMBERS BACnetARRAY), ACTION (Table 12-12), and STAGES /
///   STAGE_NAMES / TARGET_REFERENCES (Table 12-80).
/// - **Type-dependent** identifiers classify by `object_type`: ALARM_VALUES /
///   FAULT_VALUES are BACnetARRAY[N] on CharacterString Value (Table 12-44)
///   and BitString Value (Table 12-47) but BACnetLIST on the multi-state,
///   life-safety, and access families; LIST_OF_OBJECT_PROPERTY_REFERENCES is
///   BACnetARRAY[N] on Channel (Table 12-62) but BACnetLIST on Schedule
///   (Table 12-28) and Timer (Table 12-75); PRESENT_VALUE is
///   BACnetARRAY[N] of BACnetPropertyAccessResult on Global Group
///   (Table 12-57) but scalar elsewhere.
/// - **Everything else** — scalars and the identifier-stable BACnetLIST
///   properties DATE_LIST (Table 12-11), LIST_OF_GROUP_MEMBERS
///   (Table 12-17), RECIPIENT_LIST (Table 12-24), LOG_BUFFER
///   (Tables 12-29/12-31), DEVICE_ADDRESS_BINDING and
///   ACTIVE_COV_SUBSCRIPTIONS (Table 12-13) — takes no index: Clause 12.1.5.2
///   makes ReadRange the only positional access to a BACnetLIST. Array-typed
///   identifiers whose object types are not modeled in-tree (e.g.
///   ACTION_TEXT, EVENT_MESSAGE_TEXTS_CONFIG, VALUE_SOURCE_ARRAY) stay
///   rejected until their object-side modeling lands.
///
/// Like [`historical_writable_default`] this is a free function (not a
/// per-object override) so the default trait method can delegate to it
/// without requiring `Self: Sized` (which would break `dyn BACnetObject`
/// dispatch).
#[inline]
pub(crate) fn array_property_default(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> bool {
    match property {
        PropertyIdentifier::OBJECT_LIST
        | PropertyIdentifier::PROPERTY_LIST
        | PropertyIdentifier::STATE_TEXT
        | PropertyIdentifier::PRIORITY
        | PropertyIdentifier::WEEKLY_SCHEDULE
        | PropertyIdentifier::EXCEPTION_SCHEDULE
        | PropertyIdentifier::EVENT_TIME_STAMPS
        | PropertyIdentifier::EVENT_MESSAGE_TEXTS
        | PropertyIdentifier::PRIORITY_ARRAY
        | PropertyIdentifier::TAGS
        | PropertyIdentifier::SUBORDINATE_LIST
        | PropertyIdentifier::SUBORDINATE_ANNOTATIONS
        | PropertyIdentifier::GROUP_MEMBERS
        | PropertyIdentifier::GROUP_MEMBER_NAMES
        | PropertyIdentifier::ACTION
        | PropertyIdentifier::STAGES
        | PropertyIdentifier::STAGE_NAMES
        | PropertyIdentifier::TARGET_REFERENCES => true,
        PropertyIdentifier::ALARM_VALUES | PropertyIdentifier::FAULT_VALUES => matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
            object_type == ObjectType::CHANNEL
        }
        PropertyIdentifier::PRESENT_VALUE => object_type == ObjectType::GLOBAL_GROUP,
        _ => false,
    }
}

/// The historical PICS writable-property heuristic, used by the default
/// [`BACnetObject::is_writable_property`] so unmigrated object types keep
/// their current PICS output.
///
/// This is a free function (not a per-object override) so the default trait
/// method can delegate to it without requiring `Self: Sized` (which would
/// break `dyn BACnetObject` dispatch). Object implementations should override
/// [`BACnetObject::is_writable_property`] to mirror their real `write_property`
/// arms exactly rather than calling this.
#[inline]
pub(crate) fn historical_writable_default(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> bool {
    // Universal read-only properties.
    if property == PropertyIdentifier::OBJECT_IDENTIFIER
        || property == PropertyIdentifier::OBJECT_TYPE
        || property == PropertyIdentifier::PROPERTY_LIST
        || property == PropertyIdentifier::STATUS_FLAGS
    {
        return false;
    }

    if property == PropertyIdentifier::OBJECT_NAME {
        return true;
    }

    if property == PropertyIdentifier::PRESENT_VALUE {
        return object_type != ObjectType::ANALOG_INPUT
            && object_type != ObjectType::BINARY_INPUT
            && object_type != ObjectType::MULTI_STATE_INPUT;
    }

    property == PropertyIdentifier::DESCRIPTION
        || property == PropertyIdentifier::OUT_OF_SERVICE
        || property == PropertyIdentifier::COV_INCREMENT
        || property == PropertyIdentifier::HIGH_LIMIT
        || property == PropertyIdentifier::LOW_LIMIT
        || property == PropertyIdentifier::DEADBAND
        || property == PropertyIdentifier::NOTIFICATION_CLASS
}
