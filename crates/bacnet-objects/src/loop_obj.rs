//! Loop (type 12) object per ASHRAE 135-2020 Clause 12.19.
//!
//! PID control loop. The application is responsible for running the PID
//! algorithm; this object stores configuration and current output.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_property_list_property};
use crate::traits::BACnetObject;

/// BACnet Loop object — PID control loop configuration and state.
pub struct LoopObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    setpoint: f32,
    proportional_constant: f32,
    integral_constant: f32,
    derivative_constant: f32,
    output_units: u32,
    update_interval: u32,
    out_of_service: bool,
    reliability: u32,
    status_flags: StatusFlags,
    controlled_variable_reference: Option<BACnetObjectPropertyReference>,
    manipulated_variable_reference: Option<BACnetObjectPropertyReference>,
    setpoint_reference: Option<BACnetObjectPropertyReference>,
}

impl LoopObject {
    pub fn new(instance: u32, name: impl Into<String>, output_units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LOOP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            setpoint: 0.0,
            proportional_constant: 1.0,
            integral_constant: 0.0,
            derivative_constant: 0.0,
            output_units,
            update_interval: 1000, // milliseconds
            out_of_service: false,
            reliability: 0,
            status_flags: StatusFlags::empty(),
            controlled_variable_reference: None,
            manipulated_variable_reference: None,
            setpoint_reference: None,
        })
    }

    /// Application sets the current output value after PID computation.
    pub fn set_present_value(&mut self, value: f32) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the controlled variable reference (the object whose present value is
    /// being controlled by this loop).
    pub fn set_controlled_variable_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.controlled_variable_reference = Some(r);
    }

    /// Set the manipulated variable reference (the object that the loop output
    /// drives to achieve the setpoint).
    pub fn set_manipulated_variable_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.manipulated_variable_reference = Some(r);
    }

    /// Set the setpoint reference (an alternative way to supply the setpoint
    /// from another object's property instead of the inline `SETPOINT` value).
    pub fn set_setpoint_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.setpoint_reference = Some(r);
    }
}

impl BACnetObject for LoopObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LOOP.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::SETPOINT => Ok(PropertyValue::Real(self.setpoint)),
            p if p == PropertyIdentifier::PROPORTIONAL_CONSTANT => {
                Ok(PropertyValue::Real(self.proportional_constant))
            }
            p if p == PropertyIdentifier::INTEGRAL_CONSTANT => {
                Ok(PropertyValue::Real(self.integral_constant))
            }
            p if p == PropertyIdentifier::DERIVATIVE_CONSTANT => {
                Ok(PropertyValue::Real(self.derivative_constant))
            }
            p if p == PropertyIdentifier::OUTPUT_UNITS => {
                Ok(PropertyValue::Enumerated(self.output_units))
            }
            p if p == PropertyIdentifier::UPDATE_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.update_interval as u64))
            }
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability))
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE => {
                match &self.controlled_variable_reference {
                    Some(r) => Ok(PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(r.object_identifier),
                        PropertyValue::Enumerated(r.property_identifier),
                    ])),
                    None => Ok(PropertyValue::Null),
                }
            }
            p if p == PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE => {
                match &self.manipulated_variable_reference {
                    Some(r) => Ok(PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(r.object_identifier),
                        PropertyValue::Enumerated(r.property_identifier),
                    ])),
                    None => Ok(PropertyValue::Null),
                }
            }
            p if p == PropertyIdentifier::SETPOINT_REFERENCE => match &self.setpoint_reference {
                Some(r) => Ok(PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(r.object_identifier),
                    PropertyValue::Enumerated(r.property_identifier),
                ])),
                None => Ok(PropertyValue::Null),
            },
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        match property {
            p if p == PropertyIdentifier::SETPOINT => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.setpoint = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::PROPORTIONAL_CONSTANT => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.proportional_constant = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::INTEGRAL_CONSTANT => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.integral_constant = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::DERIVATIVE_CONSTANT => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.derivative_constant = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::UPDATE_INTERVAL => {
                if let PropertyValue::Unsigned(v) = value {
                    self.update_interval = common::u64_to_u32(v)?;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                if let PropertyValue::Enumerated(v) = value {
                    self.reliability = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                if let PropertyValue::Boolean(v) = value {
                    self.out_of_service = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                if let PropertyValue::CharacterString(s) = value {
                    self.description = s;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE => match value {
                PropertyValue::Null => {
                    self.controlled_variable_reference = None;
                    Ok(())
                }
                PropertyValue::List(ref items) if items.len() >= 2 => {
                    if let (PropertyValue::ObjectIdentifier(oid), PropertyValue::Enumerated(prop)) =
                        (&items[0], &items[1])
                    {
                        self.controlled_variable_reference =
                            Some(BACnetObjectPropertyReference::new(*oid, *prop));
                        return Ok(());
                    }
                    Err(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw() as u32,
                        code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                    })
                }
                _ => Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                }),
            },
            p if p == PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE => match value {
                PropertyValue::Null => {
                    self.manipulated_variable_reference = None;
                    Ok(())
                }
                PropertyValue::List(ref items) if items.len() >= 2 => {
                    if let (PropertyValue::ObjectIdentifier(oid), PropertyValue::Enumerated(prop)) =
                        (&items[0], &items[1])
                    {
                        self.manipulated_variable_reference =
                            Some(BACnetObjectPropertyReference::new(*oid, *prop));
                        return Ok(());
                    }
                    Err(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw() as u32,
                        code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                    })
                }
                _ => Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                }),
            },
            p if p == PropertyIdentifier::SETPOINT_REFERENCE => match value {
                PropertyValue::Null => {
                    self.setpoint_reference = None;
                    Ok(())
                }
                PropertyValue::List(ref items) if items.len() >= 2 => {
                    if let (PropertyValue::ObjectIdentifier(oid), PropertyValue::Enumerated(prop)) =
                        (&items[0], &items[1])
                    {
                        self.setpoint_reference =
                            Some(BACnetObjectPropertyReference::new(*oid, *prop));
                        return Ok(());
                    }
                    Err(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw() as u32,
                        code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                    })
                }
                _ => Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                }),
            },
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            }),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::SETPOINT,
            PropertyIdentifier::PROPORTIONAL_CONSTANT,
            PropertyIdentifier::INTEGRAL_CONSTANT,
            PropertyIdentifier::DERIVATIVE_CONSTANT,
            PropertyIdentifier::OUTPUT_UNITS,
            PropertyIdentifier::UPDATE_INTERVAL,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
            PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE,
            PropertyIdentifier::SETPOINT_REFERENCE,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_read_defaults() {
        let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        assert_eq!(
            lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::SETPOINT, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::PROPORTIONAL_CONSTANT, None)
                .unwrap(),
            PropertyValue::Real(1.0)
        );
    }

    #[test]
    fn loop_write_pid_constants() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        lo.write_property(
            PropertyIdentifier::SETPOINT,
            None,
            PropertyValue::Real(72.0),
            None,
        )
        .unwrap();
        lo.write_property(
            PropertyIdentifier::PROPORTIONAL_CONSTANT,
            None,
            PropertyValue::Real(2.5),
            None,
        )
        .unwrap();
        lo.write_property(
            PropertyIdentifier::INTEGRAL_CONSTANT,
            None,
            PropertyValue::Real(0.1),
            None,
        )
        .unwrap();
        lo.write_property(
            PropertyIdentifier::DERIVATIVE_CONSTANT,
            None,
            PropertyValue::Real(0.05),
            None,
        )
        .unwrap();

        assert_eq!(
            lo.read_property(PropertyIdentifier::SETPOINT, None)
                .unwrap(),
            PropertyValue::Real(72.0)
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::PROPORTIONAL_CONSTANT, None)
                .unwrap(),
            PropertyValue::Real(2.5)
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::INTEGRAL_CONSTANT, None)
                .unwrap(),
            PropertyValue::Real(0.1)
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::DERIVATIVE_CONSTANT, None)
                .unwrap(),
            PropertyValue::Real(0.05)
        );
    }

    #[test]
    fn loop_set_present_value() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        lo.set_present_value(55.0);
        assert_eq!(
            lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(55.0)
        );
    }

    #[test]
    fn loop_read_object_type() {
        let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let val = lo
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(ObjectType::LOOP.to_raw()));
    }

    #[test]
    fn loop_write_wrong_type_rejected() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let result = lo.write_property(
            PropertyIdentifier::SETPOINT,
            None,
            PropertyValue::Unsigned(72),
            None,
        );
        assert!(result.is_err());
    }

    // --- Property reference tests ---

    #[test]
    fn loop_references_default_to_null() {
        let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        assert_eq!(
            lo.read_property(PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
        assert_eq!(
            lo.read_property(PropertyIdentifier::SETPOINT_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn loop_set_controlled_variable_reference_read_back() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        lo.set_controlled_variable_reference(BACnetObjectPropertyReference::new(oid, prop_raw));

        let val = lo
            .read_property(PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE, None)
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
    fn loop_set_manipulated_variable_reference_read_back() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap();
        let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        lo.set_manipulated_variable_reference(BACnetObjectPropertyReference::new(oid, prop_raw));

        let val = lo
            .read_property(PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE, None)
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
    fn loop_set_setpoint_reference_read_back() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 10).unwrap();
        let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
        lo.set_setpoint_reference(BACnetObjectPropertyReference::new(oid, prop_raw));

        let val = lo
            .read_property(PropertyIdentifier::SETPOINT_REFERENCE, None)
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
    fn loop_references_in_property_list() {
        let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let list = lo.property_list();
        assert!(list.contains(&PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE));
        assert!(list.contains(&PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE));
        assert!(list.contains(&PropertyIdentifier::SETPOINT_REFERENCE));
    }

    #[test]
    fn loop_write_reference_via_write_property() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
        let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();

        lo.write_property(
            PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
            None,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Enumerated(prop_raw),
            ]),
            None,
        )
        .unwrap();

        assert_eq!(
            lo.read_property(PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Enumerated(prop_raw),
            ])
        );
    }

    #[test]
    fn loop_write_null_clears_reference() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        lo.set_controlled_variable_reference(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));

        // Verify it is set
        assert_ne!(
            lo.read_property(PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );

        // Write Null to clear
        lo.write_property(
            PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();

        assert_eq!(
            lo.read_property(PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE, None)
                .unwrap(),
            PropertyValue::Null
        );
    }

    #[test]
    fn loop_write_reference_wrong_type_rejected() {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let result = lo.write_property(
            PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
            None,
            PropertyValue::Unsigned(42),
            None,
        );
        assert!(result.is_err());
    }
}
