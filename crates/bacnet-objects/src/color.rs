//! Color (type 63) and Color Temperature (type 64) objects.
//!
//! Per ASHRAE 135-2020 Addendum bj, Clauses 12.55-12.56.
//!
//! Color objects represent CIE 1931 xy color coordinates.
//! Color Temperature objects represent correlated color temperature in Kelvin.
//! Both support fade transitions via Color_Command.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties, read_property_list_property};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// ColorObject (type 63) — CIE 1931 xy color
// ---------------------------------------------------------------------------

/// BACnet Color object (type 63).
///
/// Represents a color as CIE 1931 xy coordinates. Supports FADE_TO_COLOR
/// transitions via Color_Command. Non-commandable (no priority array).
pub struct ColorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value: BACnetxyColor encoded as (x: REAL, y: REAL).
    /// Stored as two f32 values.
    present_value_x: f32,
    present_value_y: f32,
    /// Tracking_Value: current actual color (may differ during fade).
    tracking_value_x: f32,
    tracking_value_y: f32,
    /// Color_Command: last written command (opaque bytes for now).
    color_command: Vec<u8>,
    /// Default_Color: startup color (x, y).
    default_color_x: f32,
    default_color_y: f32,
    /// Default_Fade_Time: milliseconds (100-86400000). 0 = use device default.
    default_fade_time: u32,
    /// Transition: 0=NONE, 1=FADE.
    transition: u32,
    /// In_Progress: 0=idle, 1=fade-active.
    in_progress: u32,
    status_flags: StatusFlags,
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
}

impl ColorObject {
    /// Create a new Color object with default white color (x=0.3127, y=0.3290 ≈ D65).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COLOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value_x: 0.3127,
            present_value_y: 0.3290,
            tracking_value_x: 0.3127,
            tracking_value_y: 0.3290,
            color_command: Vec::new(),
            default_color_x: 0.3127,
            default_color_y: 0.3290,
            default_fade_time: 0,
            transition: 0,  // NONE
            in_progress: 0, // idle
            status_flags: StatusFlags::empty(),
            event_state: 0, // NORMAL
            out_of_service: false,
            reliability: 0,
        })
    }

    pub fn set_present_value(&mut self, x: f32, y: f32) {
        self.present_value_x = x;
        self.present_value_y = y;
        self.tracking_value_x = x;
        self.tracking_value_y = y;
    }
}

impl BACnetObject for ColorObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::COLOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                // BACnetxyColor encoded as a list of two REALs
                Ok(PropertyValue::List(vec![
                    PropertyValue::Real(self.present_value_x),
                    PropertyValue::Real(self.present_value_y),
                ]))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => Ok(PropertyValue::List(vec![
                PropertyValue::Real(self.tracking_value_x),
                PropertyValue::Real(self.tracking_value_y),
            ])),
            p if p == PropertyIdentifier::COLOR_COMMAND => {
                Ok(PropertyValue::OctetString(self.color_command.clone()))
            }
            p if p == PropertyIdentifier::DEFAULT_COLOR => Ok(PropertyValue::List(vec![
                PropertyValue::Real(self.default_color_x),
                PropertyValue::Real(self.default_color_y),
            ])),
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                Ok(PropertyValue::Unsigned(self.default_fade_time as u64))
            }
            p if p == PropertyIdentifier::TRANSITION => {
                Ok(PropertyValue::Enumerated(self.transition))
            }
            p if p == PropertyIdentifier::IN_PROGRESS => {
                Ok(PropertyValue::Enumerated(self.in_progress))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::COLOR_COMMAND => {
                if let PropertyValue::OctetString(data) = value {
                    self.color_command = data;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                if let PropertyValue::Unsigned(v) = value {
                    if v > 86_400_000 {
                        return Err(common::value_out_of_range_error());
                    }
                    self.default_fade_time = v as u32;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::COLOR_COMMAND,
            PropertyIdentifier::IN_PROGRESS,
            PropertyIdentifier::DEFAULT_COLOR,
            PropertyIdentifier::DEFAULT_FADE_TIME,
            PropertyIdentifier::TRANSITION,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// ColorTemperatureObject (type 64) — Correlated Color Temperature
// ---------------------------------------------------------------------------

/// BACnet Color Temperature object (type 64).
///
/// Represents correlated color temperature in Kelvin (typically 1000-30000).
/// Supports FADE, RAMP, and STEP transitions via Color_Command.
pub struct ColorTemperatureObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value: Unsigned (Kelvin).
    present_value: u32,
    /// Tracking_Value: current actual color temperature.
    tracking_value: u32,
    /// Color_Command: last written command.
    color_command: Vec<u8>,
    /// Default_Color_Temperature: startup value.
    default_color_temperature: u32,
    /// Default_Fade_Time: milliseconds.
    default_fade_time: u32,
    /// Default_Ramp_Rate: Kelvin per second.
    default_ramp_rate: u32,
    /// Default_Step_Increment: Kelvin per step.
    default_step_increment: u32,
    /// Transition: 0=NONE, 1=FADE, 2=RAMP.
    transition: u32,
    /// In_Progress: 0=idle, 1=fade-active, 2=ramp-active.
    in_progress: u32,
    /// Min/Max present value bounds.
    min_pres_value: Option<u32>,
    max_pres_value: Option<u32>,
    status_flags: StatusFlags,
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
}

impl ColorTemperatureObject {
    /// Create a new Color Temperature object with default 4000K (neutral white).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 4000,
            tracking_value: 4000,
            color_command: Vec::new(),
            default_color_temperature: 4000,
            default_fade_time: 0,
            default_ramp_rate: 100,     // 100K/s
            default_step_increment: 50, // 50K per step
            transition: 0,              // NONE
            in_progress: 0,             // idle
            min_pres_value: Some(1000),
            max_pres_value: Some(30000),
            status_flags: StatusFlags::empty(),
            event_state: 0,
            out_of_service: false,
            reliability: 0,
        })
    }

    pub fn set_present_value(&mut self, kelvin: u32) {
        self.present_value = kelvin;
        self.tracking_value = kelvin;
    }

    pub fn set_min_max(&mut self, min: u32, max: u32) {
        self.min_pres_value = Some(min);
        self.max_pres_value = Some(max);
    }
}

impl BACnetObject for ColorTemperatureObject {
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
                ObjectType::COLOR_TEMPERATURE.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Unsigned(self.tracking_value as u64))
            }
            p if p == PropertyIdentifier::COLOR_COMMAND => {
                Ok(PropertyValue::OctetString(self.color_command.clone()))
            }
            p if p == PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE => Ok(PropertyValue::Unsigned(
                self.default_color_temperature as u64,
            )),
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                Ok(PropertyValue::Unsigned(self.default_fade_time as u64))
            }
            p if p == PropertyIdentifier::DEFAULT_RAMP_RATE => {
                Ok(PropertyValue::Unsigned(self.default_ramp_rate as u64))
            }
            p if p == PropertyIdentifier::DEFAULT_STEP_INCREMENT => {
                Ok(PropertyValue::Unsigned(self.default_step_increment as u64))
            }
            p if p == PropertyIdentifier::TRANSITION => {
                Ok(PropertyValue::Enumerated(self.transition))
            }
            p if p == PropertyIdentifier::IN_PROGRESS => {
                Ok(PropertyValue::Enumerated(self.in_progress))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => match self.min_pres_value {
                Some(v) => Ok(PropertyValue::Unsigned(v as u64)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::MAX_PRES_VALUE => match self.max_pres_value {
                Some(v) => Ok(PropertyValue::Unsigned(v as u64)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Unsigned(v) = value {
                    let v32 = v as u32;
                    // Clamp to min/max if supported
                    if let Some(min) = self.min_pres_value {
                        if v32 < min {
                            return Err(common::value_out_of_range_error());
                        }
                    }
                    if let Some(max) = self.max_pres_value {
                        if v32 > max {
                            return Err(common::value_out_of_range_error());
                        }
                    }
                    self.present_value = v32;
                    self.tracking_value = v32;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::COLOR_COMMAND => {
                if let PropertyValue::OctetString(data) = value {
                    self.color_command = data;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::COLOR_COMMAND,
            PropertyIdentifier::IN_PROGRESS,
            PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE,
            PropertyIdentifier::DEFAULT_FADE_TIME,
            PropertyIdentifier::DEFAULT_RAMP_RATE,
            PropertyIdentifier::DEFAULT_STEP_INCREMENT,
            PropertyIdentifier::TRANSITION,
            PropertyIdentifier::MIN_PRES_VALUE,
            PropertyIdentifier::MAX_PRES_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}
