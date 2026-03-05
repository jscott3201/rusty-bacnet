//! Staging object (type 60) per ASHRAE 135-2020 Clause 12.
//!
//! The Staging object organizes a set of stages (numbered 0..N-1) with
//! human-readable names and target references. Present value indicates
//! the current active stage.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Staging object — manages a set of named stages.
pub struct StagingObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u64,
    stage_names: Vec<String>,
    target_references: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl StagingObject {
    /// Create a new Staging object with the given number of stages.
    ///
    /// Stage names default to "Stage 0", "Stage 1", etc.
    pub fn new(instance: u32, name: impl Into<String>, num_stages: usize) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::STAGING, instance)?;
        let stage_names = (0..num_stages).map(|i| format!("Stage {i}")).collect();
        let target_references = vec![Vec::new(); num_stages];
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            stage_names,
            target_references,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set a stage name by index.
    pub fn set_stage_name(&mut self, index: usize, name: impl Into<String>) {
        if index < self.stage_names.len() {
            self.stage_names[index] = name.into();
        }
    }
}

impl BACnetObject for StagingObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::STAGING.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value))
            }
            p if p == PropertyIdentifier::STAGE_NAMES => {
                let items: Vec<PropertyValue> = self
                    .stage_names
                    .iter()
                    .map(|s| PropertyValue::CharacterString(s.clone()))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::TARGET_REFERENCES => {
                let items: Vec<PropertyValue> = self
                    .target_references
                    .iter()
                    .map(|r| PropertyValue::OctetString(r.clone()))
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
                    if v as usize >= self.stage_names.len() {
                        return Err(common::value_out_of_range_error());
                    }
                    self.present_value = v;
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
            PropertyIdentifier::STAGE_NAMES,
            PropertyIdentifier::TARGET_REFERENCES,
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
    fn staging_create_and_read_defaults() {
        let stg = StagingObject::new(1, "STG-1", 3).unwrap();
        assert_eq!(stg.object_name(), "STG-1");
        assert_eq!(
            stg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn staging_object_type() {
        let stg = StagingObject::new(1, "STG-1", 3).unwrap();
        assert_eq!(
            stg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::STAGING.to_raw())
        );
    }

    #[test]
    fn staging_read_stage_names() {
        let stg = StagingObject::new(1, "STG-1", 3).unwrap();
        let val = stg
            .read_property(PropertyIdentifier::STAGE_NAMES, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("Stage 0".into()),
                PropertyValue::CharacterString("Stage 1".into()),
                PropertyValue::CharacterString("Stage 2".into()),
            ])
        );
    }

    #[test]
    fn staging_set_stage_name() {
        let mut stg = StagingObject::new(1, "STG-1", 2).unwrap();
        stg.set_stage_name(0, "Off");
        stg.set_stage_name(1, "On");
        let val = stg
            .read_property(PropertyIdentifier::STAGE_NAMES, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("Off".into()),
                PropertyValue::CharacterString("On".into()),
            ])
        );
    }

    #[test]
    fn staging_write_present_value() {
        let mut stg = StagingObject::new(1, "STG-1", 3).unwrap();
        stg.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
        assert_eq!(
            stg.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
    }

    #[test]
    fn staging_write_present_value_out_of_range() {
        let mut stg = StagingObject::new(1, "STG-1", 3).unwrap();
        let result = stg.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(5),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn staging_write_present_value_wrong_type() {
        let mut stg = StagingObject::new(1, "STG-1", 3).unwrap();
        let result = stg.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn staging_read_target_references() {
        let stg = StagingObject::new(1, "STG-1", 2).unwrap();
        let val = stg
            .read_property(PropertyIdentifier::TARGET_REFERENCES, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::OctetString(vec![]),
                PropertyValue::OctetString(vec![]),
            ])
        );
    }

    #[test]
    fn staging_property_list() {
        let stg = StagingObject::new(1, "STG-1", 3).unwrap();
        let list = stg.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::STAGE_NAMES));
        assert!(list.contains(&PropertyIdentifier::TARGET_REFERENCES));
    }
}
