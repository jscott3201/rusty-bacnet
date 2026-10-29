//! Load Control object (type 28) per ASHRAE 135-2020 Clause 12.28.
//!
//! The Load Control object provides a standard interface for demand-response
//! load shedding. It tracks requested, expected, and actual shed levels.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_encoding::constructed::{decode_shed_level, encode_shed_level};
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use bytes::BytesMut;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod metadata;
#[cfg(test)]
mod tests;

/// BACnet Load Control object — demand-response load shedding.
///
/// Requested_Shed_Level, Expected_Shed_Level and Actual_Shed_Level are
/// [`BACnetShedLevel`] values and go out in that CHOICE's context-tagged form
/// (Clause 21). The object runs no shed state machine: Start_Time has no write
/// route (#1092), so Present_Value never leaves SHED_INACTIVE, and while it is
/// there Clauses 12.28.16 and 12.28.17 pin the expected and actual levels to
/// the Table 12-33 default of the choice the requested level uses.
pub struct LoadControlObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value: enumerated shed state (0=shed-inactive, 1=shed-request-pending,
    /// 2=shed-compliant, 3=shed-non-compliant).
    present_value: u32,
    requested_shed_level: BACnetShedLevel,
    expected_shed_level: BACnetShedLevel,
    actual_shed_level: BACnetShedLevel,
    shed_duration: u64,
    start_time: (Date, Time),
    status_flags: StatusFlags,
    /// Event_State.
    event_state: EventState,
    reliability: Reliability,
}

impl LoadControlObject {
    /// Create a new Load Control object with default values.
    ///
    /// The three shed levels start at level 0: LEVEL is the one choice every
    /// Load Control must take (Clause 12.28.10), and 0 is its no-shed default.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            requested_shed_level: BACnetShedLevel::Level(0),
            expected_shed_level: BACnetShedLevel::Level(0),
            actual_shed_level: BACnetShedLevel::Level(0),
            shed_duration: 0,
            start_time: (
                Date {
                    year: 0xFF,
                    month: 0xFF,
                    day: 0xFF,
                    day_of_week: 0xFF,
                },
                Time {
                    hour: 0xFF,
                    minute: 0xFF,
                    second: 0xFF,
                    hundredths: 0xFF,
                },
            ),
            status_flags: StatusFlags::empty(),
            event_state: EventState::NORMAL,
            reliability: Reliability::NO_FAULT_DETECTED,
        })
    }

    /// Set Requested_Shed_Level, with the checks and effects of a
    /// WriteProperty of it.
    ///
    /// A shed request asks for less load, so a percent above 100, or an amount
    /// that is negative or not finite, would ask for more than the baseline
    /// (Table 12-33) and fails with PROPERTY / VALUE_OUT_OF_RANGE, changing
    /// nothing. Any level is accepted. As the object stays SHED_INACTIVE,
    /// Expected_Shed_Level and Actual_Shed_Level then take the default of the
    /// new level's choice: 100 for a percent, 0 for a level, 0.0 for an amount.
    pub fn set_requested_shed_level(&mut self, level: BACnetShedLevel) -> Result<(), Error> {
        match level {
            BACnetShedLevel::Percent(percent) if percent > 100 => {
                return Err(common::value_out_of_range_error());
            }
            BACnetShedLevel::Amount(amount) => {
                common::reject_non_finite(amount)?;
                if amount < 0.0 {
                    return Err(common::value_out_of_range_error());
                }
            }
            _ => {}
        }
        let default = choice_default(&level);
        self.expected_shed_level = default.clone();
        self.actual_shed_level = default;
        self.requested_shed_level = level;
        Ok(())
    }

    /// Set Actual_Shed_Level, the shed the application reports achieving.
    ///
    /// It is in Requested_Shed_Level's units (Clause 12.28.17), so a level of
    /// another choice, or an amount that isn't finite, fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing.
    pub fn set_actual_shed_level(&mut self, level: BACnetShedLevel) -> Result<(), Error> {
        if core::mem::discriminant(&level) != core::mem::discriminant(&self.requested_shed_level) {
            return Err(common::value_out_of_range_error());
        }
        if let BACnetShedLevel::Amount(amount) = level {
            common::reject_non_finite(amount)?;
        }
        self.actual_shed_level = level;
        Ok(())
    }
}

/// The Table 12-33 default for the choice `level` uses, the value that also
/// cancels a shed request when written.
fn choice_default(level: &BACnetShedLevel) -> BACnetShedLevel {
    match level {
        BACnetShedLevel::Percent(_) => BACnetShedLevel::Percent(100),
        BACnetShedLevel::Level(_) => BACnetShedLevel::Level(0),
        BACnetShedLevel::Amount(_) => BACnetShedLevel::Amount(0.0),
    }
}

/// A shed level in its Clause 21 CHOICE form.
fn shed_level_value(level: &BACnetShedLevel) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_shed_level(&mut buf, level);
    PropertyValue::ApplicationData(buf.to_vec())
}

/// Decode a Requested_Shed_Level write: exactly one `BACnetShedLevel` CHOICE,
/// which the WriteProperty decoder passes on as `ApplicationData`. Any other
/// value, the bare Unsigned or REAL and the one-element list taken before
/// #1133 included, fails with PROPERTY / INVALID_DATA_TYPE.
fn decode_shed_level_write(value: PropertyValue) -> Result<BACnetShedLevel, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    match decode_shed_level(&bytes, 0) {
        Ok((level, end)) if end == bytes.len() => Ok(level),
        _ => Err(common::invalid_data_type_error()),
    }
}

impl BACnetObject for LoadControlObject {
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
        // Table 12-32 has no Out_Of_Service (#1064), and Clause 12.28 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LOAD_CONTROL.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                Ok(shed_level_value(&self.requested_shed_level))
            }
            p if p == PropertyIdentifier::EXPECTED_SHED_LEVEL => {
                Ok(shed_level_value(&self.expected_shed_level))
            }
            p if p == PropertyIdentifier::ACTUAL_SHED_LEVEL => {
                Ok(shed_level_value(&self.actual_shed_level))
            }
            p if p == PropertyIdentifier::SHED_DURATION => {
                Ok(PropertyValue::Unsigned(self.shed_duration))
            }
            p if p == PropertyIdentifier::START_TIME => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.start_time.0),
                PropertyValue::Time(self.start_time.1),
            ])),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
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
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::SHED_DURATION => {
                if let PropertyValue::Unsigned(v) = value {
                    self.shed_duration = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                self.set_requested_shed_level(decode_shed_level_write(value)?)
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    /// Table 13-1 lists Load Control, so it takes SubscribeCOV; the shed
    /// rows it reports come from the trait default.
    fn supports_cov(&self) -> bool {
        true
    }
}
