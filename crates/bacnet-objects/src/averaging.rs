//! Averaging (type 18) object per ASHRAE 135-2020 Clause 12.4.
//!
//! Computes running statistics (min, max, average) over sampled values from
//! a referenced object property.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Averaging object (type 18).
///
/// Accumulates sample values and computes min/max/average statistics.
/// The `present_value` property reflects the current average.
pub struct AveragingObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    minimum_value: f32,
    maximum_value: f32,
    average_value: f32,
    attempted_samples: u32,
    valid_samples: u32,
    object_property_reference: Option<BACnetObjectPropertyReference>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AveragingObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AVERAGING, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            minimum_value: f32::MAX,
            maximum_value: f32::MIN,
            average_value: 0.0,
            attempted_samples: 0,
            valid_samples: 0,
            object_property_reference: None,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a sample value, updating min/max/average and counts.
    pub fn add_sample(&mut self, value: f32) {
        self.attempted_samples += 1;
        self.valid_samples += 1;

        if value < self.minimum_value {
            self.minimum_value = value;
        }
        if value > self.maximum_value {
            self.maximum_value = value;
        }

        // Running average: avg = avg_prev + (value - avg_prev) / n
        self.average_value += (value - self.average_value) / self.valid_samples as f32;
        self.present_value = self.average_value;
    }

    /// Set the object property reference (the property being averaged).
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetObjectPropertyReference>,
    ) {
        self.object_property_reference = reference;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for AveragingObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::MINIMUM_VALUE => {
                if self.valid_samples == 0 {
                    Ok(PropertyValue::Real(0.0))
                } else {
                    Ok(PropertyValue::Real(self.minimum_value))
                }
            }
            p if p == PropertyIdentifier::MAXIMUM_VALUE => {
                if self.valid_samples == 0 {
                    Ok(PropertyValue::Real(0.0))
                } else {
                    Ok(PropertyValue::Real(self.maximum_value))
                }
            }
            p if p == PropertyIdentifier::AVERAGE_VALUE => {
                Ok(PropertyValue::Real(self.average_value))
            }
            p if p == PropertyIdentifier::ATTEMPTED_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.attempted_samples as u64))
            }
            p if p == PropertyIdentifier::VALID_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.valid_samples as u64))
            }
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                match &self.object_property_reference {
                    None => Ok(PropertyValue::Null),
                    Some(r) => {
                        let mut fields = vec![
                            PropertyValue::ObjectIdentifier(r.object_identifier),
                            PropertyValue::Unsigned(r.property_identifier as u64),
                        ];
                        if let Some(idx) = r.property_array_index {
                            fields.push(PropertyValue::Unsigned(idx as u64));
                        }
                        Ok(PropertyValue::List(fields))
                    }
                }
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if property == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
            match value {
                PropertyValue::Null => {
                    self.object_property_reference = None;
                    return Ok(());
                }
                PropertyValue::List(ref items) if items.len() >= 2 => {
                    if let (PropertyValue::ObjectIdentifier(oid), PropertyValue::Unsigned(prop)) =
                        (&items[0], &items[1])
                    {
                        let array_index = if items.len() > 2 {
                            if let PropertyValue::Unsigned(idx) = &items[2] {
                                Some(*idx as u32)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        self.object_property_reference = Some(if let Some(idx) = array_index {
                            BACnetObjectPropertyReference::new_indexed(*oid, *prop as u32, idx)
                        } else {
                            BACnetObjectPropertyReference::new(*oid, *prop as u32)
                        });
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                }
                _ => return Err(common::invalid_data_type_error()),
            }
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
            PropertyIdentifier::MINIMUM_VALUE,
            PropertyIdentifier::MAXIMUM_VALUE,
            PropertyIdentifier::AVERAGE_VALUE,
            PropertyIdentifier::ATTEMPTED_SAMPLES,
            PropertyIdentifier::VALID_SAMPLES,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::EVENT_STATE,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn averaging_create() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_NAME, None)
                .unwrap(),
            PropertyValue::CharacterString("AVG-1".into())
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw())
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn averaging_add_samples() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(10.0);
        avg.add_sample(20.0);
        avg.add_sample(30.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::ATTEMPTED_SAMPLES, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::VALID_SAMPLES, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
    }

    #[test]
    fn averaging_min_max() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(15.0);
        avg.add_sample(5.0);
        avg.add_sample(25.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(5.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(25.0)
        );
    }

    #[test]
    fn averaging_average_value() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(10.0);
        avg.add_sample(20.0);
        avg.add_sample(30.0);

        let val = avg
            .read_property(PropertyIdentifier::AVERAGE_VALUE, None)
            .unwrap();
        if let PropertyValue::Real(v) = val {
            assert!((v - 20.0).abs() < 0.001);
        } else {
            panic!("Expected Real");
        }

        // present_value should equal average_value
        let pv = avg
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, val);
    }

    #[test]
    fn averaging_no_samples_defaults() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        // Before any samples, min/max return 0.0
        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn averaging_property_list() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        let props = avg.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::MINIMUM_VALUE));
        assert!(props.contains(&PropertyIdentifier::MAXIMUM_VALUE));
        assert!(props.contains(&PropertyIdentifier::AVERAGE_VALUE));
        assert!(props.contains(&PropertyIdentifier::ATTEMPTED_SAMPLES));
        assert!(props.contains(&PropertyIdentifier::VALID_SAMPLES));
        assert!(props.contains(&PropertyIdentifier::OBJECT_PROPERTY_REFERENCE));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn averaging_object_property_reference_default_null() {
        let avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn averaging_set_object_property_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(oid, pv_raw)));

        let val = avg
            .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ])
        );
    }

    #[test]
    fn averaging_write_object_property_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();

        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ]),
            None,
        )
        .unwrap();

        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
            ])
        );
    }

    #[test]
    fn averaging_write_null_clears_reference() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));

        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();

        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn averaging_write_present_value_denied() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let result = avg.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn averaging_description_read_write() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString(String::new())
        );
        avg.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Zone temperature averaging".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Zone temperature averaging".into())
        );
    }

    #[test]
    fn averaging_single_sample() {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.add_sample(42.0);

        assert_eq!(
            avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(42.0)
        );
    }
}
