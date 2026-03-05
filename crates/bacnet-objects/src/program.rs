//! Program object (type 16) per ASHRAE 135-2020 Clause 12.
//!
//! The Program object represents an application program running within
//! a BACnet device. It exposes the program's lifecycle state.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// Maximum valid program state value (unloaded = 5).
const PROGRAM_STATE_MAX: u32 = 5;

/// BACnet Program object — represents an application program lifecycle.
pub struct ProgramObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    program_state: u32,
    program_change: u32,
    reason_for_halt: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl ProgramObject {
    /// Create a new Program object in the idle state.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::PROGRAM, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            program_state: 0, // idle
            program_change: 0,
            reason_for_halt: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the program state directly (application use).
    pub fn set_program_state(&mut self, state: u32) {
        self.program_state = state;
    }

    /// Set the reason for halt.
    pub fn set_reason_for_halt(&mut self, reason: u32) {
        self.reason_for_halt = reason;
    }
}

impl BACnetObject for ProgramObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::PROGRAM.to_raw()))
            }
            p if p == PropertyIdentifier::PROGRAM_STATE => {
                Ok(PropertyValue::Enumerated(self.program_state))
            }
            p if p == PropertyIdentifier::PROGRAM_CHANGE => {
                Ok(PropertyValue::Enumerated(self.program_change))
            }
            p if p == PropertyIdentifier::REASON_FOR_HALT => {
                Ok(PropertyValue::Enumerated(self.reason_for_halt))
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
            p if p == PropertyIdentifier::PROGRAM_CHANGE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.program_change = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::PROGRAM_STATE => {
                if let PropertyValue::Enumerated(v) = value {
                    if v > PROGRAM_STATE_MAX {
                        return Err(common::value_out_of_range_error());
                    }
                    self.program_state = v;
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
            PropertyIdentifier::PROGRAM_STATE,
            PropertyIdentifier::PROGRAM_CHANGE,
            PropertyIdentifier::REASON_FOR_HALT,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_create_and_read_defaults() {
        let prog = ProgramObject::new(1, "PRG-1").unwrap();
        assert_eq!(prog.object_name(), "PRG-1");
        assert_eq!(
            prog.read_property(PropertyIdentifier::PROGRAM_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            prog.read_property(PropertyIdentifier::PROGRAM_CHANGE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            prog.read_property(PropertyIdentifier::REASON_FOR_HALT, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn program_object_type() {
        let prog = ProgramObject::new(1, "PRG-1").unwrap();
        assert_eq!(
            prog.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::PROGRAM.to_raw())
        );
    }

    #[test]
    fn program_write_program_state() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        prog.write_property(
            PropertyIdentifier::PROGRAM_STATE,
            None,
            PropertyValue::Enumerated(2),
            None,
        )
        .unwrap();
        assert_eq!(
            prog.read_property(PropertyIdentifier::PROGRAM_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(2)
        );
    }

    #[test]
    fn program_write_program_state_out_of_range() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        let result = prog.write_property(
            PropertyIdentifier::PROGRAM_STATE,
            None,
            PropertyValue::Enumerated(99),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn program_write_program_state_wrong_type() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        let result = prog.write_property(
            PropertyIdentifier::PROGRAM_STATE,
            None,
            PropertyValue::Unsigned(2),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn program_write_program_change() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        prog.write_property(
            PropertyIdentifier::PROGRAM_CHANGE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        assert_eq!(
            prog.read_property(PropertyIdentifier::PROGRAM_CHANGE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    #[test]
    fn program_set_program_state() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        prog.set_program_state(4);
        assert_eq!(
            prog.read_property(PropertyIdentifier::PROGRAM_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(4)
        );
    }

    #[test]
    fn program_set_reason_for_halt() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        prog.set_reason_for_halt(3);
        assert_eq!(
            prog.read_property(PropertyIdentifier::REASON_FOR_HALT, None)
                .unwrap(),
            PropertyValue::Enumerated(3)
        );
    }

    #[test]
    fn program_property_list() {
        let prog = ProgramObject::new(1, "PRG-1").unwrap();
        let list = prog.property_list();
        assert!(list.contains(&PropertyIdentifier::PROGRAM_STATE));
        assert!(list.contains(&PropertyIdentifier::PROGRAM_CHANGE));
        assert!(list.contains(&PropertyIdentifier::REASON_FOR_HALT));
    }

    #[test]
    fn program_write_description() {
        let mut prog = ProgramObject::new(1, "PRG-1").unwrap();
        prog.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Test program".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            prog.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Test program".into())
        );
    }
}
