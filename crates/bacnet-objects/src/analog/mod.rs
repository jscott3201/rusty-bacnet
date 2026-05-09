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
        // IN_ALARM: override STATUS_FLAGS with event_state before common macro
        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                self.event_detector.event_state.to_raw(),
            ));
        }
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
        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                self.event_detector.event_state.to_raw(),
            ));
        }
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
    /// Value source tracking.
    value_source: common::ValueSourceTracking,
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
            value_source: common::ValueSourceTracking::default(),
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
        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                self.event_detector.event_state.to_raw(),
            ));
        }
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
