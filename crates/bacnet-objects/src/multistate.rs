//! Multi-State Input (type 13), Multi-State Output (type 14), and
//! Multi-State Value (type 19) objects per ASHRAE 135-2020 Clauses 12.20-12.22.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::event::{ChangeOfStateDetector, EventStateChange};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// MultiStateInput (type 13)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Input object.
///
/// Read-only multi-state point. Present_Value is writable only when out-of-service.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    state_text: Vec<String>,
    /// Alarm_Values — state values that trigger OFFNORMAL (Clause 12.18).
    alarm_values: Vec<u32>,
    /// Fault_Values — state values that indicate a fault (Clause 12.18).
    fault_values: Vec<u32>,
    /// CHANGE_OF_STATE event detector (Clause 13.3.1).
    event_detector: ChangeOfStateDetector,
}

impl MultiStateInputObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: 0,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            alarm_values: Vec::new(),
            fault_values: Vec::new(),
            event_detector: ChangeOfStateDetector::default(),
        })
    }

    /// Set the alarm values (states that trigger OFFNORMAL).
    pub fn set_alarm_values(&mut self, values: Vec<u32>) {
        self.alarm_values = values.clone();
        self.event_detector.alarm_values = values;
    }

    /// Set the fault values (states that indicate a fault).
    pub fn set_fault_values(&mut self, values: Vec<u32>) {
        self.fault_values = values;
    }

    /// Set the present value (used by application to update input state).
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for MultiStateInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
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
                ObjectType::MULTI_STATE_INPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            p if p == PropertyIdentifier::STATE_TEXT => match array_index {
                None => Ok(PropertyValue::List(
                    self.state_text
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(self.state_text.len() as u64)),
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => Ok(
                    PropertyValue::CharacterString(self.state_text[(idx - 1) as usize].clone()),
                ),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ALARM_VALUES => Ok(PropertyValue::List(
                self.alarm_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::FAULT_VALUES => Ok(PropertyValue::List(
                self.fault_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
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
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Unsigned(v) = value {
                if v < 1 || v > self.number_of_states as u64 {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v as u32;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::STATE_TEXT {
            match array_index {
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => {
                    if let PropertyValue::CharacterString(s) = value {
                        self.state_text[(idx - 1) as usize] = s;
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                }
                None => return Err(common::write_access_denied_error()),
                _ => return Err(common::invalid_array_index_error()),
            }
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
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATE_TEXT,
            PropertyIdentifier::ALARM_VALUES,
            PropertyIdentifier::FAULT_VALUES,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// MultiStateOutput (type 14)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Output object.
///
/// Commandable multi-state output with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    state_text: Vec<String>,
    alarm_values: Vec<u32>,
    fault_values: Vec<u32>,
    /// CHANGE_OF_STATE event detector (Clause 13.3.1).
    event_detector: ChangeOfStateDetector,
}

impl MultiStateOutputObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 1,
            reliability: 0,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            alarm_values: Vec::new(),
            fault_values: Vec::new(),
            event_detector: ChangeOfStateDetector::default(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for MultiStateOutputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
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
                ObjectType::MULTI_STATE_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: u32| PropertyValue::Unsigned(
                    v as u64
                ))
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Unsigned(self.relinquish_default as u64))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p if p == PropertyIdentifier::STATE_TEXT => match array_index {
                None => Ok(PropertyValue::List(
                    self.state_text
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(self.state_text.len() as u64)),
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => Ok(
                    PropertyValue::CharacterString(self.state_text[(idx - 1) as usize].clone()),
                ),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ALARM_VALUES => Ok(PropertyValue::List(
                self.alarm_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::FAULT_VALUES => Ok(PropertyValue::List(
                self.fault_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
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
        {
            let num_states = self.number_of_states;
            common::write_priority_array_direct!(self, property, array_index, value, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            let num_states = self.number_of_states;
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::STATE_TEXT {
            match array_index {
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => {
                    if let PropertyValue::CharacterString(s) = value {
                        self.state_text[(idx - 1) as usize] = s;
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                }
                None => return Err(common::write_access_denied_error()),
                _ => return Err(common::invalid_array_index_error()),
            }
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
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATE_TEXT,
            PropertyIdentifier::ALARM_VALUES,
            PropertyIdentifier::FAULT_VALUES,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// MultiStateValue (type 19)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Value object.
///
/// Commandable multi-state value with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    state_text: Vec<String>,
    alarm_values: Vec<u32>,
    fault_values: Vec<u32>,
    /// CHANGE_OF_STATE event detector (Clause 13.3.1).
    event_detector: ChangeOfStateDetector,
}

impl MultiStateValueObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 1,
            reliability: 0,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            alarm_values: Vec::new(),
            fault_values: Vec::new(),
            event_detector: ChangeOfStateDetector::default(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for MultiStateValueObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<EventStateChange> {
        self.event_detector.evaluate(self.present_value)
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
                ObjectType::MULTI_STATE_VALUE.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: u32| PropertyValue::Unsigned(
                    v as u64
                ))
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Unsigned(self.relinquish_default as u64))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p if p == PropertyIdentifier::STATE_TEXT => match array_index {
                None => Ok(PropertyValue::List(
                    self.state_text
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(self.state_text.len() as u64)),
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => Ok(
                    PropertyValue::CharacterString(self.state_text[(idx - 1) as usize].clone()),
                ),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ALARM_VALUES => Ok(PropertyValue::List(
                self.alarm_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::FAULT_VALUES => Ok(PropertyValue::List(
                self.fault_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
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
        {
            let num_states = self.number_of_states;
            common::write_priority_array_direct!(self, property, array_index, value, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            let num_states = self.number_of_states;
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::STATE_TEXT {
            match array_index {
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => {
                    if let PropertyValue::CharacterString(s) = value {
                        self.state_text[(idx - 1) as usize] = s;
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                }
                None => return Err(common::write_access_denied_error()),
                _ => return Err(common::invalid_array_index_error()),
            }
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
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATE_TEXT,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- MultiStateInput ---

    #[test]
    fn msi_read_present_value_default() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1));
    }

    #[test]
    fn msi_read_number_of_states() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::NUMBER_OF_STATES, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(4));
    }

    #[test]
    fn msi_write_denied_when_in_service() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        let result = msi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn msi_write_allowed_when_out_of_service() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        msi.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        msi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        let val = msi
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(3));
    }

    #[test]
    fn msi_write_out_of_range_rejected() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        msi.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        assert!(msi
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(0),
                None
            )
            .is_err());
        assert!(msi
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(5),
                None
            )
            .is_err());
    }

    #[test]
    fn msi_read_reliability_default() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- MultiStateOutput ---

    #[test]
    fn mso_write_with_priority() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        mso.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(3),
            Some(8),
        )
        .unwrap();
        let val = mso
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(3));
        let slot = mso
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Unsigned(3));
    }

    #[test]
    fn mso_relinquish_falls_to_default() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        mso.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(4),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(4)
        );
        mso.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();
        assert_eq!(
            mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        ); // default
    }

    #[test]
    fn mso_out_of_range_rejected() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        assert!(mso
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(0),
                None
            )
            .is_err());
        assert!(mso
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(6),
                None
            )
            .is_err());
    }

    #[test]
    fn mso_read_reliability_default() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        let val = mso
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- MultiStateValue ---

    #[test]
    fn msv_read_present_value_default() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1));
    }

    #[test]
    fn msv_write_with_priority() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        msv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            Some(8),
        )
        .unwrap();
        let val = msv
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(2));
        let slot = msv
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Unsigned(2));
    }

    #[test]
    fn msv_relinquish_falls_to_default() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        msv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(3),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        msv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        ); // relinquish_default
    }

    #[test]
    fn msv_read_priority_array_all_none() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
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
    fn msv_read_relinquish_default() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1));
    }

    #[test]
    fn msv_write_out_of_range_rejected() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert!(msv
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(0),
                None
            )
            .is_err());
        assert!(msv
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(4),
                None
            )
            .is_err());
    }

    #[test]
    fn msv_write_wrong_type_rejected() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let result = msv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn msv_read_object_type() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::MULTI_STATE_VALUE.to_raw())
        );
    }

    #[test]
    fn msv_read_reliability_default() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- MultiStateValue direct PRIORITY_ARRAY writes (Clause 15.9.1.1.3) ---

    #[test]
    fn msv_direct_priority_array_write_value() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            msv.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
    }

    #[test]
    fn msv_direct_priority_array_relinquish() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        ); // relinquish_default
    }

    #[test]
    fn msv_direct_priority_array_no_index_error() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        let result = msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn msv_direct_priority_array_index_zero_error() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        let result = msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn msv_direct_priority_array_index_17_error() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        let result = msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn msv_direct_priority_array_range_validation() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
        // Value 0 is out of range (valid: 1..=5)
        assert!(msv
            .write_property(
                PropertyIdentifier::PRIORITY_ARRAY,
                Some(1),
                PropertyValue::Unsigned(0),
                None
            )
            .is_err());
        // Value 6 is out of range
        assert!(msv
            .write_property(
                PropertyIdentifier::PRIORITY_ARRAY,
                Some(1),
                PropertyValue::Unsigned(6),
                None
            )
            .is_err());
        // Value 5 is valid
        msv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(1),
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
    }

    // --- Direct PRIORITY_ARRAY writes (Clause 15.9.1.1.3) ---

    #[test]
    fn mso_direct_priority_array_write_value() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        assert_eq!(
            mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
    }

    #[test]
    fn mso_direct_priority_array_relinquish() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        // Fall back to relinquish default (1)
        assert_eq!(
            mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
    }

    #[test]
    fn mso_direct_priority_array_no_index_error() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        let result = mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn mso_direct_priority_array_index_zero_error() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        let result = mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn mso_direct_priority_array_index_17_error() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        let result = mso.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Unsigned(3),
            None,
        );
        assert!(result.is_err());
    }

    // --- state_text tests ---

    #[test]
    fn msi_state_text_defaults() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::STATE_TEXT, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("State 1".into()),
                PropertyValue::CharacterString("State 2".into()),
                PropertyValue::CharacterString("State 3".into()),
            ])
        );
    }

    #[test]
    fn msi_state_text_index_zero_returns_length() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(4));
    }

    #[test]
    fn msi_state_text_valid_index() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        let val = msi
            .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("State 2".into()));
    }

    #[test]
    fn msi_state_text_invalid_index_error() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        assert!(msi
            .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
            .is_err());
        assert!(msi
            .read_property(PropertyIdentifier::STATE_TEXT, Some(100))
            .is_err());
    }

    #[test]
    fn msi_state_text_write_at_index() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(2),
            PropertyValue::CharacterString("Occupied".into()),
            None,
        )
        .unwrap();
        let val = msi
            .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("Occupied".into()));
    }

    #[test]
    fn msi_state_text_write_wrong_type_rejected() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        assert!(msi
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(1),
                PropertyValue::Unsigned(42),
                None,
            )
            .is_err());
    }

    #[test]
    fn msi_state_text_write_bad_index_rejected() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        // index 0 is invalid for write
        assert!(msi
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(0),
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
        // out-of-range index
        assert!(msi
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(4),
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
        // no index
        assert!(msi
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                None,
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
    }

    #[test]
    fn msi_state_text_in_property_list() {
        let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        assert!(msi
            .property_list()
            .contains(&PropertyIdentifier::STATE_TEXT));
    }

    #[test]
    fn mso_state_text_defaults() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 2).unwrap();
        let val = mso
            .read_property(PropertyIdentifier::STATE_TEXT, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("State 1".into()),
                PropertyValue::CharacterString("State 2".into()),
            ])
        );
    }

    #[test]
    fn mso_state_text_index_zero_returns_length() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
        let val = mso
            .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(5));
    }

    #[test]
    fn mso_state_text_valid_index() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        let val = mso
            .read_property(PropertyIdentifier::STATE_TEXT, Some(3))
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("State 3".into()));
    }

    #[test]
    fn mso_state_text_invalid_index_error() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        assert!(mso
            .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
            .is_err());
    }

    #[test]
    fn mso_state_text_write_at_index() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        mso.write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(1),
            PropertyValue::CharacterString("Low".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            mso.read_property(PropertyIdentifier::STATE_TEXT, Some(1))
                .unwrap(),
            PropertyValue::CharacterString("Low".into())
        );
    }

    #[test]
    fn mso_state_text_in_property_list() {
        let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        assert!(mso
            .property_list()
            .contains(&PropertyIdentifier::STATE_TEXT));
    }

    #[test]
    fn msv_state_text_defaults() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::STATE_TEXT, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("State 1".into()),
                PropertyValue::CharacterString("State 2".into()),
                PropertyValue::CharacterString("State 3".into()),
            ])
        );
    }

    #[test]
    fn msv_state_text_index_zero_returns_length() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(3));
    }

    #[test]
    fn msv_state_text_valid_index() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let val = msv
            .read_property(PropertyIdentifier::STATE_TEXT, Some(1))
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("State 1".into()));
    }

    #[test]
    fn msv_state_text_invalid_index_error() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert!(msv
            .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
            .is_err());
        assert!(msv
            .read_property(PropertyIdentifier::STATE_TEXT, Some(0xFF))
            .is_err());
    }

    #[test]
    fn msv_state_text_write_at_index() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        msv.write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(2),
            PropertyValue::CharacterString("Comfort".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::STATE_TEXT, Some(2))
                .unwrap(),
            PropertyValue::CharacterString("Comfort".into())
        );
    }

    #[test]
    fn msv_state_text_write_bad_index_rejected() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert!(msv
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                None,
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
        assert!(msv
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(0),
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
        assert!(msv
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(4),
                PropertyValue::CharacterString("X".into()),
                None,
            )
            .is_err());
    }

    #[test]
    fn msv_state_text_write_wrong_type_rejected() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert!(msv
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(1),
                PropertyValue::Unsigned(1),
                None,
            )
            .is_err());
    }

    #[test]
    fn msv_state_text_in_property_list() {
        let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert!(msv
            .property_list()
            .contains(&PropertyIdentifier::STATE_TEXT));
    }
}
