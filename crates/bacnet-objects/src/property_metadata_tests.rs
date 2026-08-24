use std::borrow::Cow;
use std::collections::HashSet;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::analog::AnalogInputObject;
use crate::binary::BinaryInputObject;
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};
use crate::traits::BACnetObject;
use crate::value_types::{DateValueObject, TimeValueObject};

struct InstanceMetadataObject {
    oid: ObjectIdentifier,
    include_description: bool,
}

impl InstanceMetadataObject {
    fn new(instance: u32, include_description: bool) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            include_description,
        }
    }
}

impl BACnetObject for InstanceMetadataObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "instance-metadata"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                self.object_identifier().object_type().to_raw(),
            )),
            PropertyIdentifier::DESCRIPTION if self.include_description => {
                Ok(PropertyValue::CharacterString(String::new()))
            }
            PropertyIdentifier::PROPERTY_LIST => {
                crate::common::read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(crate::common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(crate::common::write_access_denied_error())
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = vec![
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_NAME,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_TYPE,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
        ];
        if self.include_description {
            rows.push(PropertyMetadata::new(
                PropertyIdentifier::DESCRIPTION,
                PropertyConformance::Optional,
                None,
                PropertyWriteCapability::ReadOnly,
            ));
        }
        rows.push(PropertyMetadata::new(
            PropertyIdentifier::PROPERTY_LIST,
            PropertyConformance::RequiredRead,
            None,
            PropertyWriteCapability::ReadOnly,
        ));
        Cow::Owned(rows)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let metadata = self.property_metadata();
        crate::property_metadata::property_list_from_metadata(metadata.as_ref())
    }
}

fn metadata_row(
    object: &dyn BACnetObject,
    property_identifier: PropertyIdentifier,
) -> PropertyMetadata {
    *object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == property_identifier)
        .expect("property should have a metadata row")
}

fn assert_unique_and_canonical(object: &dyn BACnetObject) {
    let metadata = object.property_metadata();
    assert!(!metadata.is_empty());
    assert!(metadata
        .iter()
        .any(|row| row.property_identifier == PropertyIdentifier::PROPERTY_LIST));
    let unique: HashSet<_> = metadata.iter().map(|row| row.property_identifier).collect();
    assert_eq!(
        unique.len(),
        metadata.len(),
        "duplicate metadata identifier"
    );
}

#[test]
fn property_metadata_contract_time_value() {
    let object = TimeValueObject::new(1, "TV-1").unwrap();
    assert_unique_and_canonical(&object);
    assert_eq!(object.property_metadata().len(), 11);

    let present_value = metadata_row(&object, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(present_value.conformance, PropertyConformance::RequiredRead);
    assert_eq!(present_value.presence_condition, None);
    assert_eq!(
        present_value.write_capability,
        PropertyWriteCapability::Always
    );

    let priority_array = metadata_row(&object, PropertyIdentifier::PRIORITY_ARRAY);
    assert_eq!(priority_array.conformance, PropertyConformance::Optional);
    assert_eq!(
        priority_array.presence_condition,
        Some(PropertyPresenceCondition::Commandable)
    );
    assert_eq!(
        priority_array.write_capability,
        PropertyWriteCapability::Always
    );

    let status_flags = metadata_row(&object, PropertyIdentifier::STATUS_FLAGS);
    assert_eq!(status_flags.conformance, PropertyConformance::RequiredRead);
    assert_eq!(
        status_flags.write_capability,
        PropertyWriteCapability::ReadOnly
    );
}

#[test]
fn property_metadata_contract_binary_input() {
    let object = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_unique_and_canonical(&object);
    assert_eq!(object.property_metadata().len(), 23);

    let present_value = metadata_row(&object, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(present_value.conformance, PropertyConformance::RequiredRead);
    assert_eq!(
        present_value.write_capability,
        PropertyWriteCapability::WhenOutOfService
    );

    let reliability = metadata_row(&object, PropertyIdentifier::RELIABILITY);
    assert_eq!(reliability.conformance, PropertyConformance::Optional);
    assert_eq!(
        reliability.write_capability,
        PropertyWriteCapability::WhenOutOfService
    );

    let event_enable = metadata_row(&object, PropertyIdentifier::EVENT_ENABLE);
    assert_eq!(event_enable.conformance, PropertyConformance::Optional);
    assert_eq!(
        event_enable.presence_condition,
        Some(PropertyPresenceCondition::IntrinsicReporting)
    );
    assert_eq!(
        event_enable.write_capability,
        PropertyWriteCapability::Always
    );

    for property_identifier in [
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyIdentifier::INACTIVE_TEXT,
    ] {
        assert_eq!(
            metadata_row(&object, property_identifier).presence_condition,
            Some(PropertyPresenceCondition::PairedText)
        );
    }
}

#[test]
fn property_metadata_contract_all_migrated_rows_are_readable() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(TimeValueObject::new(1, "TV-1").unwrap()),
        Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
    ];

    for object in objects {
        let metadata = object.property_metadata();
        for row in metadata.iter() {
            assert!(
                object.read_property(row.property_identifier, None).is_ok(),
                "{:?} must read {:?} without an array index",
                object.object_identifier().object_type(),
                row.property_identifier
            );
        }
    }
}

#[test]
fn property_metadata_contract_property_list_projection_excludes_property_list() {
    let cases: [(Box<dyn BACnetObject>, &[PropertyIdentifier]); 2] = [
        (
            Box::new(TimeValueObject::new(1, "TV-1").unwrap()),
            &[
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyIdentifier::RELIABILITY,
                PropertyIdentifier::PRIORITY_ARRAY,
                PropertyIdentifier::RELINQUISH_DEFAULT,
            ],
        ),
        (
            Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
            &[
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                PropertyIdentifier::EVENT_ENABLE,
                PropertyIdentifier::TIME_DELAY,
                PropertyIdentifier::TIME_DELAY_NORMAL,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyIdentifier::NOTIFICATION_CLASS,
                PropertyIdentifier::ACKED_TRANSITIONS,
                PropertyIdentifier::EVENT_TIME_STAMPS,
                PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyIdentifier::POLARITY,
                PropertyIdentifier::RELIABILITY,
                PropertyIdentifier::ACTIVE_TEXT,
                PropertyIdentifier::INACTIVE_TEXT,
                PropertyIdentifier::ALARM_VALUE,
            ],
        ),
    ];

    for (object, expected_projection) in cases {
        let projected: Vec<_> = object
            .property_metadata()
            .iter()
            .filter_map(|row| {
                (row.property_identifier != PropertyIdentifier::PROPERTY_LIST)
                    .then_some(row.property_identifier)
            })
            .collect();
        assert_eq!(projected, expected_projection);
        assert_eq!(object.property_list().as_ref(), expected_projection);
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::PROPERTY_LIST));

        let wire_list = object
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap();
        let expected_wire = PropertyValue::List(
            expected_projection
                .iter()
                .filter(|property_identifier| {
                    !matches!(
                        **property_identifier,
                        PropertyIdentifier::OBJECT_IDENTIFIER
                            | PropertyIdentifier::OBJECT_NAME
                            | PropertyIdentifier::OBJECT_TYPE
                            | PropertyIdentifier::PROPERTY_LIST
                    )
                })
                .map(|property_identifier| PropertyValue::Enumerated(property_identifier.to_raw()))
                .collect(),
        );
        assert_eq!(wire_list, expected_wire);
    }
}

#[test]
fn property_metadata_contract_macro_opt_in_and_legacy_default() {
    let time_value = TimeValueObject::new(1, "TV-1").unwrap();
    let date_value = DateValueObject::new(1, "DV-1").unwrap();
    let analog_input = AnalogInputObject::new(1, "AI-1", 62).unwrap();

    assert!(!time_value.property_metadata().is_empty());
    assert!(date_value.property_metadata().is_empty());
    assert!(analog_input.property_metadata().is_empty());
}

#[test]
fn property_metadata_contract_write_capabilities_match_dispatch() {
    let mut time_value = TimeValueObject::new(1, "TV-1").unwrap();
    assert_eq!(
        metadata_row(&time_value, PropertyIdentifier::OBJECT_NAME).write_capability,
        PropertyWriteCapability::Always
    );
    assert!(time_value
        .write_property(
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("TV-2".into()),
            None,
        )
        .is_ok());
    assert_eq!(
        metadata_row(&time_value, PropertyIdentifier::STATUS_FLAGS).write_capability,
        PropertyWriteCapability::ReadOnly
    );
    assert!(time_value
        .write_property(
            PropertyIdentifier::STATUS_FLAGS,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .is_err());

    let mut binary_input = BinaryInputObject::new(1, "BI-1").unwrap();
    for property_identifier in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert_eq!(
            metadata_row(&binary_input, property_identifier).write_capability,
            PropertyWriteCapability::WhenOutOfService
        );
    }
    assert!(binary_input
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .is_err());
    assert!(binary_input
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .is_err());
    binary_input
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    binary_input
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
    binary_input
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
}

#[test]
fn property_metadata_contract_models_required_write_code() {
    let row = PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyConformance::RequiredWrite,
        None,
        PropertyWriteCapability::Always,
    );
    assert!(row.conformance.is_required());
    assert!(row.write_capability.is_writable());
}

#[test]
fn property_metadata_contract_dyn_object_can_return_owned_instance_rows() {
    let without_description = InstanceMetadataObject::new(1, false);
    let with_description = InstanceMetadataObject::new(2, true);
    let cases: [(&dyn BACnetObject, bool); 2] =
        [(&without_description, false), (&with_description, true)];

    for (object, expect_description) in cases {
        let metadata = object.property_metadata();
        assert!(matches!(&metadata, Cow::Owned(_)));
        assert_eq!(
            metadata
                .iter()
                .any(|row| row.property_identifier == PropertyIdentifier::DESCRIPTION),
            expect_description
        );
    }
}
