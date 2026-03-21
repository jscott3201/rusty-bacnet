//! BACnetObject trait — the interface all BACnet objects implement.

use std::borrow::Cow;

use bacnet_types::constructed::BACnetLogRecord;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::event::EventStateChange;

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
    /// Returns `Some(EventStateChange)` if the event state transitioned,
    /// or `None` if no change occurred (or the object doesn't support intrinsic reporting).
    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
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

    /// Add a trend log record (only meaningful for TrendLog / TrendLogMultiple).
    ///
    /// Default is a no-op. TrendLog objects override to append to their buffer.
    fn add_trend_record(&mut self, _record: BACnetLogRecord) {}
}
