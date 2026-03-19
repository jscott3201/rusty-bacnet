//! Analog Input (type 0), Analog Output (type 1), and Analog Value (type 2) objects.
//!
//! Per ASHRAE 135-2020 Clauses 12.1 (AI), 12.2 (AO), and 12.3 (AV).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties, read_event_properties, write_event_properties};
use crate::event::{EventStateChange, OutOfRangeDetector};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// AnalogInput (type 0)
// ---------------------------------------------------------------------------

/// BACnet Analog Input object.
pub struct AnalogInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    /// Optional minimum present value for fault detection.
    min_pres_value: Option<f32>,
    /// Optional maximum present value for fault detection.
    max_pres_value: Option<f32>,
    /// Event_Time_Stamps[3]: to-offnormal, to-fault, to-normal.
    event_time_stamps: [BACnetTimeStamp; 3],
    /// Event_Message_Texts[3]: to-offnormal, to-fault, to-normal.
    event_message_texts: [String; 3],
}

impl AnalogInputObject {
    /// Create a new Analog Input object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let _oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance)?;
        Ok(Self {
            oid: _oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            cov_increment: 0.0,
            event_detector: OutOfRangeDetector::default(),
            reliability: 0,
            min_pres_value: None,
            max_pres_value: None,
            event_time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            event_message_texts: [String::new(), String::new(), String::new()],
        })
    }

    /// Set the present value (used by the application to update sensor readings).
    pub fn set_present_value(&mut self, value: f32) {
        debug_assert!(
            value.is_finite(),
            "set_present_value called with non-finite value"
        );
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the minimum present value for fault detection.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set the maximum present value for fault detection.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }
}

impl BACnetObject for AnalogInputObject {
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
        if let Some(result) = read_event_properties!(self, property) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_INPUT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => match self.min_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::MAX_PRES_VALUE => match self.max_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
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
        // AI present-value is writable only when out-of-service
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Real(v) = value {
                if !v.is_finite() {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        if let Some(result) = write_event_properties!(self, property, value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::UNITS,
            PropertyIdentifier::COV_INCREMENT,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
    }

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AnalogOutput (type 1)
// ---------------------------------------------------------------------------

/// BACnet Analog Output object.
pub struct AnalogOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    reliability: u32,
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    event_time_stamps: [BACnetTimeStamp; 3],
    event_message_texts: [String; 3],
    /// Value source tracking.
    value_source: common::ValueSourceTracking,
}

impl AnalogOutputObject {
    /// Create a new Analog Output object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0.0,
            cov_increment: 0.0,
            event_detector: OutOfRangeDetector::default(),
            reliability: 0,
            min_pres_value: None,
            max_pres_value: None,
            event_time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            event_message_texts: [String::new(), String::new(), String::new()],
            value_source: common::ValueSourceTracking::default(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the minimum present value for fault detection.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set the maximum present value for fault detection.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for AnalogOutputObject {
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
        if let Some(result) = read_event_properties!(self, property) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ANALOG_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Real)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Real(self.relinquish_default))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p if p == PropertyIdentifier::VALUE_SOURCE => {
                Ok(self.value_source.value_source.clone())
            }
            p if p == PropertyIdentifier::LAST_COMMAND_TIME => Ok(PropertyValue::Unsigned(
                match self.value_source.last_command_time {
                    BACnetTimeStamp::SequenceNumber(n) => n,
                    _ => 0,
                },
            )),
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => match self.min_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::MAX_PRES_VALUE => match self.max_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        common::write_priority_array_direct!(self, property, array_index, value, |v| {
            if let PropertyValue::Real(f) = v {
                if !f.is_finite() {
                    return Err(common::value_out_of_range_error());
                }
                Ok(f)
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Real(f) = v {
                    if !f.is_finite() {
                        return Err(common::value_out_of_range_error());
                    }
                    Ok(f)
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        if let Some(result) = write_event_properties!(self, property, value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::UNITS,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::COV_INCREMENT,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
    }

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AnalogValue (type 2)
// ---------------------------------------------------------------------------

/// BACnet Analog Value object.
pub struct AnalogValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// 16-level priority array. `None` = no command at that level.
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    event_time_stamps: [BACnetTimeStamp; 3],
    event_message_texts: [String; 3],
}

impl AnalogValueObject {
    /// Create a new Analog Value object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0.0,
            cov_increment: 0.0,
            event_detector: OutOfRangeDetector::default(),
            reliability: 0,
            min_pres_value: None,
            max_pres_value: None,
            event_time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            event_message_texts: [String::new(), String::new(), String::new()],
        })
    }

    /// Set the present value directly (bypasses priority array; use when out-of-service
    /// or for initialisation before the priority-array mechanism takes over).
    pub fn set_present_value(&mut self, value: f32) {
        debug_assert!(
            value.is_finite(),
            "set_present_value called with non-finite value"
        );
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the minimum present value for fault detection.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set the maximum present value for fault detection.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for AnalogValueObject {
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
        if let Some(result) = read_event_properties!(self, property) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Real)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Real(self.relinquish_default))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => match self.min_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::MAX_PRES_VALUE => match self.max_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        common::write_priority_array_direct!(self, property, array_index, value, |v| {
            if let PropertyValue::Real(f) = v {
                if !f.is_finite() {
                    return Err(common::value_out_of_range_error());
                }
                Ok(f)
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Real(f) = v {
                    if !f.is_finite() {
                        return Err(common::value_out_of_range_error());
                    }
                    Ok(f)
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        if let Some(result) = write_event_properties!(self, property, value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::UNITS,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::COV_INCREMENT,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
    }

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::LimitEnable;
    use bacnet_types::enums::EventState;

    // --- AnalogInput ---

    #[test]
    fn ai_read_present_value() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap(); // 62 = degrees-fahrenheit
        ai.set_present_value(72.5);
        let val = ai
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(72.5));
    }

    #[test]
    fn ai_read_units() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let val = ai.read_property(PropertyIdentifier::UNITS, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(62));
    }

    #[test]
    fn ai_write_present_value_denied_when_in_service() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let result = ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(99.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn ai_write_present_value_allowed_when_out_of_service() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(99.0),
            None,
        )
        .unwrap();
        let val = ai
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(99.0));
    }

    #[test]
    fn ai_read_unknown_property() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let result = ai.read_property(PropertyIdentifier::PRIORITY_ARRAY, None);
        assert!(result.is_err());
    }

    // --- AnalogOutput ---

    #[test]
    fn ao_write_with_priority() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

        // Write at priority 8
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(8),
        )
        .unwrap();

        let val = ao
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(50.0));

        // Priority array at index 8 should have the value
        let slot = ao
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Real(50.0));

        // Priority array at index 1 should be Null
        let slot = ao
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
            .unwrap();
        assert_eq!(slot, PropertyValue::Null);
    }

    #[test]
    fn ao_relinquish_falls_to_default() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

        // Write at priority 16 (lowest)
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(75.0),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(75.0)
        );

        // Relinquish (write Null)
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();

        // Should fall back to relinquish-default (0.0)
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn ao_higher_priority_wins() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(16),
        )
        .unwrap();
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(8),
        )
        .unwrap();

        // Priority 8 wins over 16
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(90.0)
        );
    }

    // --- Intrinsic Reporting ---

    #[test]
    fn ai_read_event_state_default_normal() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let val = ai
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(EventState::NORMAL.to_raw()));
    }

    #[test]
    fn ai_read_write_high_limit() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(85.0),
            None,
        )
        .unwrap();
        assert_eq!(
            ai.read_property(PropertyIdentifier::HIGH_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(85.0)
        );
    }

    #[test]
    fn ai_read_write_low_limit() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(15.0),
            None,
        )
        .unwrap();
        assert_eq!(
            ai.read_property(PropertyIdentifier::LOW_LIMIT, None)
                .unwrap(),
            PropertyValue::Real(15.0)
        );
    }

    #[test]
    fn ai_read_write_deadband() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.5),
            None,
        )
        .unwrap();
        assert_eq!(
            ai.read_property(PropertyIdentifier::DEADBAND, None)
                .unwrap(),
            PropertyValue::Real(2.5)
        );
    }

    #[test]
    fn ai_deadband_reject_negative() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let result = ai.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(-1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn ai_read_write_limit_enable() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let enable_both = LimitEnable::BOTH.to_bits();
        ai.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![enable_both],
            },
            None,
        )
        .unwrap();
        let val = ai
            .read_property(PropertyIdentifier::LIMIT_ENABLE, None)
            .unwrap();
        if let PropertyValue::BitString { data, .. } = val {
            let le = LimitEnable::from_bits(data[0]);
            assert!(le.low_limit_enable);
            assert!(le.high_limit_enable);
        } else {
            panic!("Expected BitString");
        }
    }

    #[test]
    fn ai_intrinsic_reporting_triggers_on_present_value_change() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        // Configure: high=80, low=20, deadband=2, both limits enabled
        ai.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x07 << 5], // all transitions enabled
            },
            None,
        )
        .unwrap();

        // Normal value — no transition
        ai.set_present_value(50.0);
        assert!(ai.evaluate_intrinsic_reporting().is_none());

        // Go above high limit
        ai.set_present_value(81.0);
        let change = ai.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.from, EventState::NORMAL);
        assert_eq!(change.to, EventState::HIGH_LIMIT);

        // Verify event_state property reads correctly
        assert_eq!(
            ai.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );

        // Drop below deadband threshold → back to NORMAL
        ai.set_present_value(77.0);
        let change = ai.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn ao_intrinsic_reporting_after_priority_write() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        ao.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        ao.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        ao.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        ao.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        ao.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x07 << 5], // all transitions enabled
            },
            None,
        )
        .unwrap();

        // Write a high value via priority array
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(85.0),
            Some(8),
        )
        .unwrap();
        let change = ao.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.to, EventState::HIGH_LIMIT);
    }

    #[test]
    fn ai_read_reliability_default() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let val = ai
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    #[test]
    fn ai_description_read_write() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        // Default description is empty
        let val = ai
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString(String::new()));
        // Write a description
        ai.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Zone temperature sensor".into()),
            None,
        )
        .unwrap();
        let val = ai
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::CharacterString("Zone temperature sensor".into())
        );
    }

    #[test]
    fn ai_set_description_convenience() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_description("Supply air temperature");
        assert_eq!(
            ai.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Supply air temperature".into())
        );
    }

    #[test]
    fn ai_description_in_property_list() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        assert!(ai
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn ao_description_read_write() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        ao.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Chilled water valve".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            ao.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Chilled water valve".into())
        );
    }

    #[test]
    fn ao_description_in_property_list() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        assert!(ao
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn ao_read_reliability_default() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        let val = ao
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- Priority array bounds tests ---

    #[test]
    fn ao_priority_array_index_zero_returns_size() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        let val = ao
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(16));
    }

    #[test]
    fn ao_priority_array_index_out_of_bounds() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Index 17 is out of bounds (valid: 0-16)
        let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
        assert!(result.is_err());
    }

    #[test]
    fn ao_priority_array_index_far_out_of_bounds() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Large index well beyond valid range
        let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(100));
        assert!(result.is_err());
    }

    #[test]
    fn ao_priority_array_index_u32_max_out_of_bounds() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(u32::MAX));
        assert!(result.is_err());
    }

    // --- WriteProperty with invalid priority tests ---

    #[test]
    fn ao_write_with_priority_zero_rejected() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Priority 0 is invalid (valid range is 1-16)
        let result = ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(0),
        );
        assert!(result.is_err());
    }

    #[test]
    fn ao_write_with_priority_17_rejected() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Priority 17 is invalid (valid range is 1-16)
        let result = ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(17),
        );
        assert!(result.is_err());
    }

    #[test]
    fn ao_write_with_priority_255_rejected() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Priority 255 is invalid
        let result = ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(255),
        );
        assert!(result.is_err());
    }

    #[test]
    fn ao_write_with_all_valid_priorities() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // All priorities 1 through 16 should succeed
        for prio in 1..=16u8 {
            ao.write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(prio as f32),
                Some(prio),
            )
            .unwrap();
        }
        // Present value should be the highest priority (priority 1)
        let val = ao
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(1.0));
    }

    #[test]
    fn ao_priority_array_read_all_slots_none_by_default() {
        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Read entire array (no index)
        let val = ao
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
            .unwrap();
        if let PropertyValue::List(elements) = val {
            assert_eq!(elements.len(), 16);
            for elem in &elements {
                assert_eq!(elem, &PropertyValue::Null);
            }
        } else {
            panic!("Expected List for priority array without index");
        }
    }

    // --- Direct PRIORITY_ARRAY writes ---

    #[test]
    fn ao_direct_priority_array_write_value() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Write directly to PRIORITY_ARRAY[5]
        ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap();
        // present_value should reflect the written value
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        // Slot 5 should have the value
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Real(42.0)
        );
    }

    #[test]
    fn ao_direct_priority_array_relinquish() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Write a value at priority 5
        ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap();
        // Relinquish with Null
        ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        // Should fall back to relinquish default (0.0)
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn ao_direct_priority_array_no_index_error() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // Writing PRIORITY_ARRAY without array_index should error
        let result = ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn ao_direct_priority_array_index_zero_error() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        let result = ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn ao_direct_priority_array_index_17_error() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        let result = ao.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    // --- AnalogValue ---

    #[test]
    fn av_read_present_value_default() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(0.0));
    }

    #[test]
    fn av_set_present_value() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.set_present_value(42.5);
        let val = av
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(42.5));
    }

    #[test]
    fn av_read_object_type_returns_analog_value() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw())
        );
    }

    #[test]
    fn av_read_units() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av.read_property(PropertyIdentifier::UNITS, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(62));
    }

    #[test]
    fn av_write_with_priority() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

        // Write at priority 8
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(55.0),
            Some(8),
        )
        .unwrap();

        let val = av
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(55.0));

        // Priority array at index 8 should have the value
        let slot = av
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Real(55.0));

        // Priority array at index 1 should be Null
        let slot = av
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
            .unwrap();
        assert_eq!(slot, PropertyValue::Null);
    }

    #[test]
    fn av_relinquish_falls_to_default() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

        // Write at priority 16 (lowest)
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(75.0),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(75.0)
        );

        // Relinquish (write Null)
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();

        // Should fall back to relinquish-default (0.0)
        assert_eq!(
            av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn av_higher_priority_wins() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(16),
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(8),
        )
        .unwrap();

        // Priority 8 wins over 16
        assert_eq!(
            av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(90.0)
        );
    }

    #[test]
    fn av_priority_array_read_all_slots_none_by_default() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
            .unwrap();
        if let PropertyValue::List(elements) = val {
            assert_eq!(elements.len(), 16);
            for elem in &elements {
                assert_eq!(elem, &PropertyValue::Null);
            }
        } else {
            panic!("Expected List for priority array without index");
        }
    }

    #[test]
    fn av_priority_array_index_zero_returns_size() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(16));
    }

    #[test]
    fn av_priority_array_index_out_of_bounds() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
        assert!(result.is_err());
    }

    #[test]
    fn av_priority_array_index_u32_max_out_of_bounds() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(u32::MAX));
        assert!(result.is_err());
    }

    #[test]
    fn av_write_with_priority_zero_rejected() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(0),
        );
        assert!(result.is_err());
    }

    #[test]
    fn av_write_with_priority_17_rejected() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(17),
        );
        assert!(result.is_err());
    }

    #[test]
    fn av_write_with_all_valid_priorities() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        for prio in 1..=16u8 {
            av.write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(prio as f32),
                Some(prio),
            )
            .unwrap();
        }
        // Present value should be the highest priority (priority 1)
        let val = av
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(1.0));
    }

    #[test]
    fn av_direct_priority_array_write_value() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Real(42.0)
        );
    }

    #[test]
    fn av_direct_priority_array_relinquish() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn av_direct_priority_array_no_index_error() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn av_direct_priority_array_index_zero_error() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn av_direct_priority_array_index_17_error() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let result = av.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn av_intrinsic_reporting_normal_to_high_limit_to_normal() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        // Configure: high=80, low=20, deadband=2, both limits enabled
        av.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x07 << 5],
            },
            None,
        )
        .unwrap();

        // Normal value — no transition
        av.set_present_value(50.0);
        assert!(av.evaluate_intrinsic_reporting().is_none());

        // Go above high limit
        av.set_present_value(81.0);
        let change = av.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.from, EventState::NORMAL);
        assert_eq!(change.to, EventState::HIGH_LIMIT);

        // Verify event_state property reads correctly
        assert_eq!(
            av.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );

        // Drop below deadband threshold → back to NORMAL
        av.set_present_value(77.0);
        let change = av.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn av_intrinsic_reporting_after_priority_write() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        av.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x07 << 5],
            },
            None,
        )
        .unwrap();

        // Write a high value via priority array
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(85.0),
            Some(8),
        )
        .unwrap();
        let change = av.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(change.to, EventState::HIGH_LIMIT);
    }

    #[test]
    fn av_read_reliability_default() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    #[test]
    fn av_description_read_write() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        // Default description is empty
        let val = av
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString(String::new()));
        // Write a description
        av.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Setpoint".into()),
            None,
        )
        .unwrap();
        let val = av
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("Setpoint".into()));
    }

    #[test]
    fn av_set_description_convenience() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.set_description("Zone temperature setpoint");
        assert_eq!(
            av.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Zone temperature setpoint".into())
        );
    }

    #[test]
    fn av_description_in_property_list() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        assert!(av
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn av_property_list_includes_priority_array_and_relinquish_default() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let list = av.property_list();
        assert!(list.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(list.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
        assert!(list.contains(&PropertyIdentifier::COV_INCREMENT));
        assert!(list.contains(&PropertyIdentifier::UNITS));
    }

    #[test]
    fn av_read_event_state_default_normal() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(EventState::NORMAL.to_raw()));
    }

    #[test]
    fn av_cov_increment_read_write() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.write_property(
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(1.5),
            None,
        )
        .unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::COV_INCREMENT, None)
                .unwrap(),
            PropertyValue::Real(1.5)
        );
        assert_eq!(av.cov_increment(), Some(1.5));
    }

    #[test]
    fn av_read_relinquish_default() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let val = av
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(0.0));
    }

    #[test]
    fn av_unknown_property_returns_error() {
        let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        // File-object property does not exist on AV
        let result = av.read_property(PropertyIdentifier::FILE_SIZE, None);
        assert!(result.is_err());
    }

    // --- PROPERTY_LIST ---

    #[test]
    fn ai_property_list_returns_full_list() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let result = ai
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap();
        if let PropertyValue::List(elements) = result {
            assert!(!elements.is_empty());
            assert!(matches!(elements[0], PropertyValue::Enumerated(_)));
        } else {
            panic!("Expected PropertyValue::List");
        }
    }

    #[test]
    fn ai_property_list_index_zero_returns_count() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        // Property_List excludes OBJECT_IDENTIFIER, OBJECT_NAME,
        // OBJECT_TYPE, and PROPERTY_LIST itself.
        let filtered_count = ai
            .property_list()
            .iter()
            .filter(|p| {
                **p != PropertyIdentifier::OBJECT_IDENTIFIER
                    && **p != PropertyIdentifier::OBJECT_NAME
                    && **p != PropertyIdentifier::OBJECT_TYPE
                    && **p != PropertyIdentifier::PROPERTY_LIST
            })
            .count() as u64;
        let result = ai
            .read_property(PropertyIdentifier::PROPERTY_LIST, Some(0))
            .unwrap();
        assert_eq!(result, PropertyValue::Unsigned(filtered_count));
    }

    #[test]
    fn ai_property_list_index_one_returns_first_prop() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        // First property after filtering the 4 excluded ones
        let first_filtered = ai
            .property_list()
            .iter()
            .copied()
            .find(|p| {
                *p != PropertyIdentifier::OBJECT_IDENTIFIER
                    && *p != PropertyIdentifier::OBJECT_NAME
                    && *p != PropertyIdentifier::OBJECT_TYPE
                    && *p != PropertyIdentifier::PROPERTY_LIST
            })
            .unwrap();
        let result = ai
            .read_property(PropertyIdentifier::PROPERTY_LIST, Some(1))
            .unwrap();
        assert_eq!(result, PropertyValue::Enumerated(first_filtered.to_raw()));
    }

    #[test]
    fn ai_property_list_invalid_index_returns_error() {
        let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let count = ai.property_list().len() as u32;
        let result = ai.read_property(PropertyIdentifier::PROPERTY_LIST, Some(count + 1));
        assert!(result.is_err());
    }
}
