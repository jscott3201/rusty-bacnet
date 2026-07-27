//! BACnetObject trait — the interface all BACnet objects implement.

use std::borrow::Cow;

use bacnet_types::constructed::BACnetLogRecord;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::event::TransitionOutcome;

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
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error>;

    /// List all properties this object supports.
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]>;

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
        historical_writable_default(self.object_identifier().object_type(), property)
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
    /// Default returns the four universal required properties.
    /// Object implementations may override to include type-specific required properties.
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
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
    /// not here.
    fn set_event_state_internal(&mut self, _state: EventState) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Add a trend log record (only meaningful for TrendLog / TrendLogMultiple).
    ///
    /// Default is a no-op. TrendLog objects override to append to their buffer.
    fn add_trend_record(&mut self, _record: BACnetLogRecord) {}
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
