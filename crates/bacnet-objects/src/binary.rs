//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.4, 12.5, 12.6.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::event::{ChangeOfStateDetector, EventStateChange};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// BinaryInput (type 3)
// ---------------------------------------------------------------------------

/// BACnet Binary Input object.
///
/// Read-only binary point. Present_Value is writable only when out-of-service.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    active_text: String,
    inactive_text: String,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
}

impl BinaryInputObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            polarity: 0,
            reliability: 0,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
            event_detector: ChangeOfStateDetector::default(),
        })
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

impl BACnetObject for BinaryInputObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_INPUT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::POLARITY => Ok(PropertyValue::Enumerated(self.polarity)),
            p if p == PropertyIdentifier::ACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.active_text.clone()))
            }
            p if p == PropertyIdentifier::INACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.inactive_text.clone()))
            }
            p if p == PropertyIdentifier::ALARM_VALUES => Ok(PropertyValue::List(
                self.event_detector
                    .alarm_values
                    .iter()
                    .map(|v| PropertyValue::Enumerated(*v))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.event_enable << 5],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.acked_transitions << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(
                self.event_detector.notification_class as u64,
            )),
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Ok(PropertyValue::List(vec![
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
            ])),
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
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Enumerated(v) = value {
                if v > 1 {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::ACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.active_text = s;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::INACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.inactive_text = s;
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
            PropertyIdentifier::POLARITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// BinaryOutput (type 4)
// ---------------------------------------------------------------------------

/// BACnet Binary Output object.
///
/// Commandable binary output with 16-level priority array.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    active_text: String,
    inactive_text: String,
    /// COMMAND_FAILURE event detector.
    event_detector: ChangeOfStateDetector,
    /// Value source tracking (optional per spec — exposed via VALUE_SOURCE property).
    value_source: common::ValueSourceTracking,
}

impl BinaryOutputObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0,
            polarity: 0,
            reliability: 0,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
            event_detector: ChangeOfStateDetector::default(),
            value_source: common::ValueSourceTracking::default(),
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

impl BACnetObject for BinaryOutputObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::BINARY_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default))
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
            p if p == PropertyIdentifier::POLARITY => Ok(PropertyValue::Enumerated(self.polarity)),
            p if p == PropertyIdentifier::ACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.active_text.clone()))
            }
            p if p == PropertyIdentifier::INACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.inactive_text.clone()))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.event_enable << 5],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.acked_transitions << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(
                self.event_detector.notification_class as u64,
            )),
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Ok(PropertyValue::List(vec![
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
            ])),
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
            if let PropertyValue::Enumerated(e) = v {
                if e > 1 {
                    Err(common::value_out_of_range_error())
                } else {
                    Ok(e)
                }
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Enumerated(e) = v {
                    if e > 1 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(e)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::ACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.active_text = s;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::INACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.inactive_text = s;
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
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::POLARITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// BinaryValue (type 5)
// ---------------------------------------------------------------------------

/// BACnet Binary Value object.
///
/// Commandable binary value with 16-level priority array.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // 0 = inactive, 1 = active
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    active_text: String,
    inactive_text: String,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Value source tracking (optional per spec — exposed via VALUE_SOURCE property).
    value_source: common::ValueSourceTracking,
}

impl BinaryValueObject {
    /// Create a new Binary Value object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // inactive
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0,
            reliability: 0,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
            event_detector: ChangeOfStateDetector::default(),
            value_source: common::ValueSourceTracking::default(),
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

impl BACnetObject for BinaryValueObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default))
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
            p if p == PropertyIdentifier::ACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.active_text.clone()))
            }
            p if p == PropertyIdentifier::INACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.inactive_text.clone()))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.event_enable << 5],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.acked_transitions << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(
                self.event_detector.notification_class as u64,
            )),
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Ok(PropertyValue::List(vec![
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(0),
            ])),
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
            if let PropertyValue::Enumerated(e) = v {
                if e > 1 {
                    Err(common::value_out_of_range_error())
                } else {
                    Ok(e)
                }
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Enumerated(e) = v {
                    if e > 1 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(e)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::ACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.active_text = s;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::INACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.inactive_text = s;
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
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bv_read_present_value_default() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let val = bv
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // inactive
    }

    #[test]
    fn bv_write_present_value() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1), // active
            Some(8),
        )
        .unwrap();
        let val = bv
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
    }

    #[test]
    fn bv_write_invalid_value_rejected() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let result = bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(2), // invalid -- only 0 or 1
            Some(8),
        );
        assert!(result.is_err());
    }

    #[test]
    fn bv_write_wrong_type_rejected() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let result = bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0), // wrong type
            Some(8),
        );
        assert!(result.is_err());
    }

    #[test]
    fn bv_read_object_type() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let val = bv
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw())
        );
    }

    #[test]
    fn bv_read_reliability_default() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let val = bv
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- BinaryInput ---

    #[test]
    fn bi_read_present_value_default() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let val = bi
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn bi_write_denied_when_in_service() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let result = bi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn bi_write_allowed_when_out_of_service() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        bi.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        bi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        let val = bi
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
    }

    #[test]
    fn bi_set_present_value() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        bi.set_present_value(1);
        let val = bi
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
    }

    #[test]
    fn bi_read_polarity_default() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let val = bi
            .read_property(PropertyIdentifier::POLARITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // normal
    }

    #[test]
    fn bi_read_reliability_default() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let val = bi
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- BinaryOutput ---

    #[test]
    fn bo_write_with_priority() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        let val = bo
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
        let slot = bo
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Enumerated(1));
    }

    #[test]
    fn bo_relinquish_falls_to_default() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
        bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn bo_invalid_value_rejected() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let result = bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(2),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn bo_read_polarity_default() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
            .read_property(PropertyIdentifier::POLARITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // normal
    }

    #[test]
    fn bo_read_reliability_default() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
    }

    // --- Priority array bounds tests (BinaryOutput) ---

    #[test]
    fn bo_priority_array_index_zero_returns_size() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(16));
    }

    #[test]
    fn bo_priority_array_index_out_of_bounds() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        // Index 17 is out of bounds (valid: 0-16)
        let result = bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
        assert!(result.is_err());
    }

    #[test]
    fn bo_priority_array_index_far_out_of_bounds() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let result = bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(100));
        assert!(result.is_err());
    }

    // --- WriteProperty with invalid priority tests (BinaryOutput) ---

    #[test]
    fn bo_write_with_priority_zero_rejected() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        // Priority 0 is invalid (valid range is 1-16)
        let result = bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(0),
        );
        assert!(result.is_err());
    }

    #[test]
    fn bo_write_with_priority_17_rejected() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        // Priority 17 is invalid (valid range is 1-16)
        let result = bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(17),
        );
        assert!(result.is_err());
    }

    #[test]
    fn bo_write_with_priority_255_rejected() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        // Priority 255 is invalid
        let result = bo.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(255),
        );
        assert!(result.is_err());
    }

    // --- BinaryInput read-only properties ---

    #[test]
    fn bi_polarity_is_readable_as_enumerated() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let val = bi
            .read_property(PropertyIdentifier::POLARITY, None)
            .unwrap();
        // Polarity default is 0 (normal), verify it comes back as Enumerated
        match val {
            PropertyValue::Enumerated(v) => assert_eq!(v, 0),
            other => panic!("Expected Enumerated for POLARITY, got {:?}", other),
        }
    }

    #[test]
    fn bi_reliability_is_readable_as_enumerated() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let val = bi
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        // Reliability default is 0 (NO_FAULT_DETECTED), verify correct type
        match val {
            PropertyValue::Enumerated(v) => assert_eq!(v, 0),
            other => panic!("Expected Enumerated for RELIABILITY, got {:?}", other),
        }
    }

    #[test]
    fn bo_polarity_is_readable_as_enumerated() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
            .read_property(PropertyIdentifier::POLARITY, None)
            .unwrap();
        match val {
            PropertyValue::Enumerated(v) => assert_eq!(v, 0),
            other => panic!("Expected Enumerated for POLARITY, got {:?}", other),
        }
    }

    #[test]
    fn bo_reliability_is_readable_as_enumerated() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        match val {
            PropertyValue::Enumerated(v) => assert_eq!(v, 0),
            other => panic!("Expected Enumerated for RELIABILITY, got {:?}", other),
        }
    }

    #[test]
    fn bi_polarity_in_property_list() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let props = bi.property_list();
        assert!(props.contains(&PropertyIdentifier::POLARITY));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn bo_priority_array_read_all_slots_none_by_default() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let val = bo
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
    fn bo_direct_priority_array_write_value() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Enumerated(1), // active
            None,
        )
        .unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
        assert_eq!(
            bo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    #[test]
    fn bo_direct_priority_array_relinquish() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        // Fall back to relinquish default (0 = inactive)
        assert_eq!(
            bo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn bo_direct_priority_array_no_index_error() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let result = bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn bo_direct_priority_array_index_zero_error() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let result = bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn bo_direct_priority_array_index_17_error() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let result = bo.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }

    // --- BinaryValue commandable tests ---

    #[test]
    fn bv_write_with_priority() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        let val = bv
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
        let slot = bv
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Enumerated(1));
    }

    #[test]
    fn bv_relinquish_falls_to_default() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(16),
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // relinquish_default
        );
    }

    #[test]
    fn bv_read_priority_array_all_none() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let val = bv
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
    fn bv_read_relinquish_default() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let val = bv
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn bv_priority_array_in_property_list() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let props = bv.property_list();
        assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    }

    #[test]
    fn bv_direct_priority_array_write() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    #[test]
    fn bv_direct_priority_array_relinquish() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        bv.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // relinquish_default
        );
    }

    // --- Description tests ---

    #[test]
    fn bv_description_read_write() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        // Default is empty
        assert_eq!(
            bv.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString(String::new())
        );
        bv.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Occupied/Unoccupied".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Occupied/Unoccupied".into())
        );
    }

    #[test]
    fn bv_description_in_property_list() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        assert!(bv
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn bi_description_read_write() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        bi.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Door contact".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bi.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Door contact".into())
        );
    }

    #[test]
    fn bo_description_read_write() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Fan enable".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Fan enable".into())
        );
    }

    #[test]
    fn bv_higher_priority_wins() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        // Write inactive at priority 10
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(0),
            Some(10),
        )
        .unwrap();
        // Write active at priority 5 (higher)
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(5),
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1) // priority 5 wins
        );
        // Relinquish priority 5, falls to priority 10
        bv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(5),
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // priority 10 value
        );
    }

    // --- active_text / inactive_text tests ---

    #[test]
    fn bi_active_inactive_text_defaults() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        assert_eq!(
            bi.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Active".into())
        );
        assert_eq!(
            bi.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Inactive".into())
        );
    }

    #[test]
    fn bi_active_inactive_text_write_read() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        bi.write_property(
            PropertyIdentifier::ACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("On".into()),
            None,
        )
        .unwrap();
        bi.write_property(
            PropertyIdentifier::INACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Off".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bi.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("On".into())
        );
        assert_eq!(
            bi.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Off".into())
        );
    }

    #[test]
    fn bi_active_text_wrong_type_rejected() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        assert!(bi
            .write_property(
                PropertyIdentifier::ACTIVE_TEXT,
                None,
                PropertyValue::Enumerated(1),
                None,
            )
            .is_err());
    }

    #[test]
    fn bi_inactive_text_wrong_type_rejected() {
        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        assert!(bi
            .write_property(
                PropertyIdentifier::INACTIVE_TEXT,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .is_err());
    }

    #[test]
    fn bi_active_inactive_text_in_property_list() {
        let bi = BinaryInputObject::new(1, "BI-1").unwrap();
        let props = bi.property_list();
        assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
        assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
    }

    #[test]
    fn bo_active_inactive_text_defaults() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Active".into())
        );
        assert_eq!(
            bo.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Inactive".into())
        );
    }

    #[test]
    fn bo_active_inactive_text_write_read() {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        bo.write_property(
            PropertyIdentifier::ACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Running".into()),
            None,
        )
        .unwrap();
        bo.write_property(
            PropertyIdentifier::INACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Stopped".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bo.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Running".into())
        );
        assert_eq!(
            bo.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Stopped".into())
        );
    }

    #[test]
    fn bo_active_inactive_text_in_property_list() {
        let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        let props = bo.property_list();
        assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
        assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
    }

    #[test]
    fn bv_active_inactive_text_defaults() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Active".into())
        );
        assert_eq!(
            bv.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Inactive".into())
        );
    }

    #[test]
    fn bv_active_inactive_text_write_read() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        bv.write_property(
            PropertyIdentifier::ACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Occupied".into()),
            None,
        )
        .unwrap();
        bv.write_property(
            PropertyIdentifier::INACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Unoccupied".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            bv.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Occupied".into())
        );
        assert_eq!(
            bv.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
                .unwrap(),
            PropertyValue::CharacterString("Unoccupied".into())
        );
    }

    #[test]
    fn bv_active_inactive_text_in_property_list() {
        let bv = BinaryValueObject::new(1, "BV-1").unwrap();
        let props = bv.property_list();
        assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
        assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
    }

    #[test]
    fn bv_active_text_wrong_type_rejected() {
        let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
        assert!(bv
            .write_property(
                PropertyIdentifier::ACTIVE_TEXT,
                None,
                PropertyValue::Real(1.0),
                None,
            )
            .is_err());
    }
}
