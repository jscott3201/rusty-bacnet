//! Accumulator (type 23) and Pulse Converter (type 24) objects.
//!
//! Per ASHRAE 135-2020 Clauses 12.1 (Accumulator) and 12.2 (PulseConverter).

use bacnet_types::constructed::{BACnetObjectPropertyReference, BACnetPrescale, BACnetScale};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// AccumulatorObject (type 23)
// ---------------------------------------------------------------------------

/// BACnet Accumulator object — tracks a pulse count with scaling.
pub struct AccumulatorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u64,
    max_pres_value: u64,
    scale: BACnetScale,
    prescale: Option<BACnetPrescale>,
    pulse_rate: f32,
    units: u32,
    limit_monitoring_interval: u32,
    status_flags: StatusFlags,
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
    value_before_change: u64,
    value_set: u64,
}

impl AccumulatorObject {
    /// Create a new Accumulator object with default values.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            max_pres_value: u64::MAX,
            scale: BACnetScale::FloatScale(1.0),
            prescale: None,
            pulse_rate: 0.0,
            units,
            limit_monitoring_interval: 0,
            status_flags: StatusFlags::empty(),
            event_state: 0,
            out_of_service: false,
            reliability: 0,
            value_before_change: 0,
            value_set: 0,
        })
    }

    /// Set the present value (application use).
    pub fn set_present_value(&mut self, value: u64) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the scale.
    pub fn set_scale(&mut self, scale: BACnetScale) {
        self.scale = scale;
    }

    /// Set the prescale.
    pub fn set_prescale(&mut self, prescale: BACnetPrescale) {
        self.prescale = Some(prescale);
    }
}

impl BACnetObject for AccumulatorObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::ACCUMULATOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value))
            }
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.max_pres_value))
            }
            p if p == PropertyIdentifier::SCALE => match &self.scale {
                BACnetScale::FloatScale(v) => {
                    Ok(PropertyValue::List(vec![PropertyValue::Real(*v)]))
                }
                BACnetScale::IntegerScale(v) => {
                    Ok(PropertyValue::List(vec![PropertyValue::Signed(*v)]))
                }
            },
            p if p == PropertyIdentifier::PRESCALE => match &self.prescale {
                Some(ps) => Ok(PropertyValue::List(vec![
                    PropertyValue::Unsigned(ps.multiplier as u64),
                    PropertyValue::Unsigned(ps.modulo_divide as u64),
                ])),
                None => Ok(PropertyValue::Null),
            },
            p if p == PropertyIdentifier::PULSE_RATE => Ok(PropertyValue::Real(self.pulse_rate)),
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::LIMIT_MONITORING_INTERVAL => Ok(PropertyValue::Unsigned(
                self.limit_monitoring_interval as u64,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::VALUE_BEFORE_CHANGE => {
                Ok(PropertyValue::Unsigned(self.value_before_change))
            }
            p if p == PropertyIdentifier::VALUE_SET => Ok(PropertyValue::Unsigned(self.value_set)),
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
        // Present value is read-only from the network
        if property == PropertyIdentifier::PRESENT_VALUE {
            return Err(common::write_access_denied_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                if let PropertyValue::Unsigned(v) = value {
                    self.max_pres_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::PULSE_RATE => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.pulse_rate = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::LIMIT_MONITORING_INTERVAL => {
                if let PropertyValue::Unsigned(v) = value {
                    self.limit_monitoring_interval = common::u64_to_u32(v)?;
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
            PropertyIdentifier::MAX_PRES_VALUE,
            PropertyIdentifier::SCALE,
            PropertyIdentifier::PRESCALE,
            PropertyIdentifier::PULSE_RATE,
            PropertyIdentifier::UNITS,
            PropertyIdentifier::LIMIT_MONITORING_INTERVAL,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::VALUE_BEFORE_CHANGE,
            PropertyIdentifier::VALUE_SET,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// PulseConverterObject (type 24)
// ---------------------------------------------------------------------------

/// BACnet Pulse Converter object — converts accumulated pulses to an analog value.
pub struct PulseConverterObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    scale_factor: f32,
    adjust_value: f32,
    cov_increment: f32,
    input_reference: Option<BACnetObjectPropertyReference>,
    status_flags: StatusFlags,
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
}

impl PulseConverterObject {
    /// Create a new Pulse Converter object with default values.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::PULSE_CONVERTER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            scale_factor: 1.0,
            adjust_value: 0.0,
            cov_increment: 0.0,
            input_reference: None,
            status_flags: StatusFlags::empty(),
            event_state: 0,
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the input reference.
    pub fn set_input_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.input_reference = Some(r);
    }
}

impl BACnetObject for PulseConverterObject {
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
                ObjectType::PULSE_CONVERTER.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::SCALE_FACTOR => {
                Ok(PropertyValue::Real(self.scale_factor))
            }
            p if p == PropertyIdentifier::ADJUST_VALUE => {
                Ok(PropertyValue::Real(self.adjust_value))
            }
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::INPUT_REFERENCE => match &self.input_reference {
                Some(r) => Ok(PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(r.object_identifier),
                    PropertyValue::Enumerated(r.property_identifier),
                ])),
                None => Ok(PropertyValue::Null),
            },
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
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
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::SCALE_FACTOR => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.scale_factor = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::ADJUST_VALUE => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.adjust_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::INPUT_REFERENCE => match value {
                PropertyValue::Null => {
                    self.input_reference = None;
                    Ok(())
                }
                PropertyValue::List(ref items) if items.len() >= 2 => {
                    if let (PropertyValue::ObjectIdentifier(oid), PropertyValue::Enumerated(prop)) =
                        (&items[0], &items[1])
                    {
                        self.input_reference =
                            Some(BACnetObjectPropertyReference::new(*oid, *prop));
                        Ok(())
                    } else {
                        Err(common::invalid_data_type_error())
                    }
                }
                _ => Err(common::invalid_data_type_error()),
            },
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
            PropertyIdentifier::UNITS,
            PropertyIdentifier::SCALE_FACTOR,
            PropertyIdentifier::ADJUST_VALUE,
            PropertyIdentifier::COV_INCREMENT,
            PropertyIdentifier::INPUT_REFERENCE,
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

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::BACnetPrescale;

    // --- AccumulatorObject ---

    #[test]
    fn accumulator_create_and_read_defaults() {
        let acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        assert_eq!(acc.object_name(), "ACC-1");
        assert_eq!(
            acc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            acc.read_property(PropertyIdentifier::UNITS, None).unwrap(),
            PropertyValue::Enumerated(95)
        );
    }

    #[test]
    fn accumulator_read_present_value() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        acc.set_present_value(42);
        assert_eq!(
            acc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(42)
        );
    }

    #[test]
    fn accumulator_present_value_read_only_from_network() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let result = acc.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(10),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn accumulator_read_scale_float() {
        let acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::List(vec![PropertyValue::Real(1.0)]));
    }

    #[test]
    fn accumulator_read_scale_integer() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        acc.set_scale(BACnetScale::IntegerScale(10));
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::List(vec![PropertyValue::Signed(10)]));
    }

    #[test]
    fn accumulator_read_prescale_none() {
        let acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let val = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Null);
    }

    #[test]
    fn accumulator_read_prescale_set() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        acc.set_prescale(BACnetPrescale {
            multiplier: 5,
            modulo_divide: 100,
        });
        let val = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::Unsigned(5),
                PropertyValue::Unsigned(100),
            ])
        );
    }

    #[test]
    fn accumulator_object_type() {
        let acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let val = acc
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::ACCUMULATOR.to_raw())
        );
    }

    #[test]
    fn accumulator_property_list() {
        let acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let list = acc.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::SCALE));
        assert!(list.contains(&PropertyIdentifier::PRESCALE));
        assert!(list.contains(&PropertyIdentifier::MAX_PRES_VALUE));
        assert!(list.contains(&PropertyIdentifier::PULSE_RATE));
    }

    // --- PulseConverterObject ---

    #[test]
    fn pulse_converter_create_and_read_defaults() {
        let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        assert_eq!(pc.object_name(), "PC-1");
        assert_eq!(
            pc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            pc.read_property(PropertyIdentifier::UNITS, None).unwrap(),
            PropertyValue::Enumerated(62)
        );
    }

    #[test]
    fn pulse_converter_read_write_present_value() {
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        pc.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(123.45),
            None,
        )
        .unwrap();
        assert_eq!(
            pc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(123.45)
        );
    }

    #[test]
    fn pulse_converter_read_scale_factor() {
        let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let val = pc
            .read_property(PropertyIdentifier::SCALE_FACTOR, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(1.0));
    }

    #[test]
    fn pulse_converter_write_scale_factor() {
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        pc.write_property(
            PropertyIdentifier::SCALE_FACTOR,
            None,
            PropertyValue::Real(2.5),
            None,
        )
        .unwrap();
        assert_eq!(
            pc.read_property(PropertyIdentifier::SCALE_FACTOR, None)
                .unwrap(),
            PropertyValue::Real(2.5)
        );
    }

    #[test]
    fn pulse_converter_cov_increment() {
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        assert_eq!(pc.cov_increment(), Some(0.0));
        pc.write_property(
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(1.5),
            None,
        )
        .unwrap();
        assert_eq!(
            pc.read_property(PropertyIdentifier::COV_INCREMENT, None)
                .unwrap(),
            PropertyValue::Real(1.5)
        );
        assert_eq!(pc.cov_increment(), Some(1.5));
    }

    #[test]
    fn pulse_converter_object_type() {
        let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let val = pc
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::PULSE_CONVERTER.to_raw())
        );
    }

    #[test]
    fn pulse_converter_write_wrong_type_rejected() {
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let result = pc.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(42),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn pulse_converter_input_reference_defaults_null() {
        let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        assert_eq!(
            pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn pulse_converter_set_input_reference() {
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
        let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        pc.set_input_reference(BACnetObjectPropertyReference::new(oid, prop_raw));
        let val = pc
            .read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Enumerated(prop_raw),
            ])
        );
    }

    #[test]
    fn pulse_converter_property_list() {
        let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let list = pc.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::SCALE_FACTOR));
        assert!(list.contains(&PropertyIdentifier::ADJUST_VALUE));
        assert!(list.contains(&PropertyIdentifier::COV_INCREMENT));
        assert!(list.contains(&PropertyIdentifier::INPUT_REFERENCE));
    }
}
