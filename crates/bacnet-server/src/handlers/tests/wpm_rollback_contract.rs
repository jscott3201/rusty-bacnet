use super::*;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::traits::WritePropertyRollback;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use std::borrow::Cow;

struct TokenBackedNameObject {
    oid: ObjectIdentifier,
    name: String,
    write_only_state: String,
}

impl TokenBackedNameObject {
    fn new() -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 98).unwrap(),
            name: "original".into(),
            write_only_state: "before".into(),
        }
    }
}

impl BACnetObject for TokenBackedNameObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::CharacterString(
                self.write_only_state.clone(),
            )),
            _ => Err(Error::Encoding("test property is not readable".into())),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::OBJECT_NAME
            && value == PropertyValue::CharacterString("renamed".into())
        {
            self.name = "renamed".into();
            return Ok(());
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(value) = value {
                self.write_only_state = value;
                return Ok(());
            }
        }
        Err(Error::Encoding("test property is not writable".into()))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::DESCRIPTION,
        ])
    }

    fn capture_write_property_rollback(
        &self,
        property: PropertyIdentifier,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::OBJECT_NAME)
            .then(|| WritePropertyRollback::new(self.name.clone()))
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        self.name = rollback.downcast::<String>()?;
        Ok(())
    }
}

#[test]
fn token_backed_name_rollback_restores_property_and_database_index() {
    let mut db = ObjectDatabase::new();
    let object = TokenBackedNameObject::new();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let failure = BinaryValueObject::new(1, "failure").unwrap();
    let failure_oid = failure.object_identifier();
    db.add(Box::new(failure)).unwrap();

    let mut renamed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut renamed, "renamed").unwrap();
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: renamed.to_vec(),
                    priority: None,
                }],
            },
            WriteAccessSpecification {
                object_identifier: failure_oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                }],
            },
        ],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());
    assert_eq!(db.get(&oid).unwrap().object_name(), "original");
    assert_eq!(
        db.find_by_name("original")
            .map(BACnetObject::object_identifier),
        Some(oid)
    );
    assert!(db.find_by_name("renamed").is_none());
}

#[test]
fn unreadable_write_reports_failed_rollback_and_residual_object() {
    let mut db = ObjectDatabase::new();
    let object = TokenBackedNameObject::new();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();

    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut changed, "changed").unwrap();
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: changed.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    let (result, residual_oids) =
        handle_write_property_multiple_with_residuals(&mut db, &request_bytes);
    assert!(result.unwrap_err().to_string().contains("rollback failed"));
    assert_eq!(residual_oids, vec![oid]);
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("changed".into())
    );
}
