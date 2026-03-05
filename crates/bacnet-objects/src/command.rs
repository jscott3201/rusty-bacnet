//! Command object (type 7) per ASHRAE 135-2020 Clause 12.
//!
//! The Command object triggers a set of actions when its present value
//! is written. Actions are stored as opaque byte vectors.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Command object — triggers a set of actions on PV write.
pub struct CommandObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u64,
    in_process: bool,
    all_writes_successful: bool,
    action: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl CommandObject {
    /// Create a new Command object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COMMAND, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            in_process: false,
            all_writes_successful: true,
            action: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the action list (opaque byte sequences).
    pub fn set_action(&mut self, action: Vec<Vec<u8>>) {
        self.action = action;
    }
}

impl BACnetObject for CommandObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::COMMAND.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value))
            }
            p if p == PropertyIdentifier::IN_PROCESS => Ok(PropertyValue::Boolean(self.in_process)),
            p if p == PropertyIdentifier::ALL_WRITES_SUCCESSFUL => {
                Ok(PropertyValue::Boolean(self.all_writes_successful))
            }
            p if p == PropertyIdentifier::ACTION => {
                let items: Vec<PropertyValue> = self
                    .action
                    .iter()
                    .map(|a| PropertyValue::OctetString(a.clone()))
                    .collect();
                Ok(PropertyValue::List(items))
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
                    self.present_value = v;
                    // In a real system this would trigger action execution
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::ACTION => {
                // ACTION is read-only from the network
                Err(common::write_access_denied_error())
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
            PropertyIdentifier::IN_PROCESS,
            PropertyIdentifier::ALL_WRITES_SUCCESSFUL,
            PropertyIdentifier::ACTION,
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
    fn command_create_and_read_defaults() {
        let cmd = CommandObject::new(1, "CMD-1").unwrap();
        assert_eq!(cmd.object_name(), "CMD-1");
        assert_eq!(
            cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            cmd.read_property(PropertyIdentifier::IN_PROCESS, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            cmd.read_property(PropertyIdentifier::ALL_WRITES_SUCCESSFUL, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn command_object_type() {
        let cmd = CommandObject::new(1, "CMD-1").unwrap();
        assert_eq!(
            cmd.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::COMMAND.to_raw())
        );
    }

    #[test]
    fn command_write_present_value() {
        let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
        cmd.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        assert_eq!(
            cmd.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
    }

    #[test]
    fn command_write_present_value_wrong_type() {
        let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
        let result = cmd.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn command_action_read_only() {
        let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
        let result = cmd.write_property(
            PropertyIdentifier::ACTION,
            None,
            PropertyValue::OctetString(vec![1, 2, 3]),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn command_read_action_empty() {
        let cmd = CommandObject::new(1, "CMD-1").unwrap();
        assert_eq!(
            cmd.read_property(PropertyIdentifier::ACTION, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn command_read_action_with_data() {
        let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
        cmd.set_action(vec![vec![1, 2, 3], vec![4, 5]]);
        assert_eq!(
            cmd.read_property(PropertyIdentifier::ACTION, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::OctetString(vec![1, 2, 3]),
                PropertyValue::OctetString(vec![4, 5]),
            ])
        );
    }

    #[test]
    fn command_property_list() {
        let cmd = CommandObject::new(1, "CMD-1").unwrap();
        let list = cmd.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::IN_PROCESS));
        assert!(list.contains(&PropertyIdentifier::ALL_WRITES_SUCCESSFUL));
        assert!(list.contains(&PropertyIdentifier::ACTION));
    }
}
