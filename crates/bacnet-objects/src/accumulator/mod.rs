//! Accumulator (type 23) and Pulse Converter (type 24) objects.
//!
//! Per ASHRAE 135-2020 §12.61 Table 12-79 (Accumulator) and §12.23 Table 12-27 (PulseConverter).

use bacnet_encoding::constructed::{encode_prescale, encode_scale};
use bacnet_types::constructed::{BACnetPrescale, BACnetScale};
use bacnet_types::enums::{
    EngineeringUnits, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod metadata;
mod pulse_converter;

#[doc(hidden)]
pub use pulse_converter::InputReading;
pub use pulse_converter::PulseConverterObject;

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
    units: EngineeringUnits,
    limit_monitoring_interval: u32,
    status_flags: StatusFlags,
    event_state: EventState,
    out_of_service: bool,
    reliability: Reliability,
    value_before_change: u64,
    value_set: u64,
}

impl AccumulatorObject {
    /// Create a new Accumulator object with default values.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
    ) -> Result<Self, Error> {
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
            event_state: EventState::NORMAL,
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
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

    /// Set the prescale. Prescale is optional (Table 12-79), and the object
    /// serves it only once this has set one: until then it is absent from
    /// Property_List and a read is UNKNOWN_PROPERTY, since BACnetPrescale
    /// has no value standing for none.
    pub fn set_prescale(&mut self, prescale: BACnetPrescale) {
        self.prescale = Some(prescale);
    }

    /// Whether Prescale is present, for the metadata.
    fn has_prescale(&self) -> bool {
        self.prescale.is_some()
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
            // Both are constructed productions (Clause 21): Scale a CHOICE of
            // a float [0] or an integer [1], Prescale its two Unsigneds under
            // [0] and [1], served as their context-tagged octets (#1487).
            p if p == PropertyIdentifier::SCALE => {
                let mut encoded = BytesMut::new();
                encode_scale(&mut encoded, &self.scale);
                Ok(PropertyValue::ApplicationData(encoded.to_vec()))
            }
            p if p == PropertyIdentifier::PRESCALE => match &self.prescale {
                Some(prescale) => {
                    let mut encoded = BytesMut::new();
                    encode_prescale(&mut encoded, prescale);
                    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
                }
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::PULSE_RATE => Ok(PropertyValue::Real(self.pulse_rate)),
            p if p == PropertyIdentifier::UNITS => {
                Ok(PropertyValue::Enumerated(self.units.to_raw()))
            }
            p if p == PropertyIdentifier::LIMIT_MONITORING_INTERVAL => Ok(PropertyValue::Unsigned(
                self.limit_monitoring_interval as u64,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
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
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_accumulator_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
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
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
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
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_present_value(42);
        assert_eq!(
            acc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(42)
        );
    }

    #[test]
    fn accumulator_present_value_read_only_from_network() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
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
        // float-scale [0], a four-octet REAL (#1487).
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(
            val,
            PropertyValue::ApplicationData(vec![0x0C, 0x3F, 0x80, 0x00, 0x00])
        );
    }

    #[test]
    fn accumulator_read_scale_integer() {
        // integer-scale [1], a signed INTEGER (#1487).
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_scale(BACnetScale::IntegerScale(10));
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::ApplicationData(vec![0x19, 0x0A]));
        acc.set_scale(BACnetScale::IntegerScale(-2));
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::ApplicationData(vec![0x19, 0xFE]));
    }

    #[test]
    fn accumulator_without_prescale_does_not_serve_it() {
        // Optional, and BACnetPrescale has no NULL: absent until set.
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        let err = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap_err();
        assert!(
            matches!(err, Error::Protocol { class, code }
                if class == bacnet_types::enums::ErrorClass::PROPERTY
                    && code == bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY),
            "{err:?}"
        );
        assert!(!acc.property_list().contains(&PropertyIdentifier::PRESCALE));
    }

    #[test]
    fn accumulator_read_prescale_set() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_prescale(BACnetPrescale {
            multiplier: 5,
            modulo_divide: 100,
        });
        let val = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap();
        // The multiplier under [0], the modulo divide under [1] (#1487).
        assert_eq!(
            val,
            PropertyValue::ApplicationData(vec![0x09, 0x05, 0x19, 0x64])
        );
    }

    #[test]
    fn accumulator_object_type() {
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
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
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_prescale(BACnetPrescale {
            multiplier: 1,
            modulo_divide: 1,
        });
        let list = acc.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::SCALE));
        assert!(list.contains(&PropertyIdentifier::PRESCALE));
        assert!(list.contains(&PropertyIdentifier::MAX_PRES_VALUE));
        assert!(list.contains(&PropertyIdentifier::PULSE_RATE));
    }
}
