//! A custom object's metadata rows state the value types of its vendor
//! properties (#2177), and the server follows them: the array-index gate,
//! the list services, ReadRange's list check and the NULL-write rule. The
//! write policy (typed list elements, references kept inside the device)
//! stays the server's own, keyed by standard type and property, so a custom
//! object's reference property passes through as written.
//!
//! The NULL rule's datatypes are pinned here end to end too: a property
//! whose datatype admits NULL (BACnetPriorityValue, BACnetChannelValue)
//! keeps the object's refusal of one, while one whose datatype has none, or
//! whose row states no datatype, takes the NULL as an unchanged success.

use super::*;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::property_metadata::{
    Datatype, PropertyConformance, PropertyMetadata, PropertyWriteCapability, ValueType,
};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use std::borrow::Cow;

const ARRAY: PropertyIdentifier = PropertyIdentifier::from_raw(600);
const LIST: PropertyIdentifier = PropertyIdentifier::from_raw(601);
const NULLABLE: PropertyIdentifier = PropertyIdentifier::from_raw(602);
const PLAIN: PropertyIdentifier = PropertyIdentifier::from_raw(603);
const UNTYPED: PropertyIdentifier = PropertyIdentifier::from_raw(604);
const REFERENCE: PropertyIdentifier = PropertyIdentifier::from_raw(605);

const fn row(property: PropertyIdentifier) -> PropertyMetadata {
    PropertyMetadata::new(
        property,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    )
}

const ROWS: &[PropertyMetadata] = &[
    row(PropertyIdentifier::OBJECT_IDENTIFIER),
    row(PropertyIdentifier::OBJECT_NAME),
    row(PropertyIdentifier::OBJECT_TYPE),
    row(PropertyIdentifier::PROPERTY_LIST),
    row(ARRAY).with_value_type(ValueType::array(Datatype::Real)),
    row(LIST).with_value_type(ValueType::list(Datatype::Unsigned)),
    row(NULLABLE).with_value_type(ValueType::single(Datatype::OptionalUnsigned)),
    row(PLAIN).with_value_type(ValueType::single(Datatype::Unsigned)),
    row(UNTYPED),
    row(REFERENCE).with_value_type(ValueType::single(Datatype::DeviceObjectPropertyReference)),
    // A standard row, which takes the standard value type.
    row(PropertyIdentifier::PRIORITY_ARRAY),
];

/// A Channel with no Priority_Array, so its Present_Value isn't commandable
/// and only its BACnetChannelValue keeps a NULL from the rule.
const CHANNEL_ROWS: &[PropertyMetadata] = &[
    row(PropertyIdentifier::OBJECT_IDENTIFIER),
    row(PropertyIdentifier::OBJECT_NAME),
    row(PropertyIdentifier::OBJECT_TYPE),
    row(PropertyIdentifier::PROPERTY_LIST),
    row(PropertyIdentifier::PRESENT_VALUE),
];

/// A vendor object. Every property refuses a NULL as a value of the wrong
/// datatype, so the NULL rule decides what the requester hears.
struct Vendor {
    oid: ObjectIdentifier,
    name: &'static str,
    rows: &'static [PropertyMetadata],
    array: Vec<PropertyValue>,
    list: Vec<PropertyValue>,
    reference: PropertyValue,
}

impl BACnetObject for Vendor {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        self.name
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(self.rows)
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            ARRAY => match index {
                None => Ok(PropertyValue::List(self.array.clone())),
                Some(0) => Ok(PropertyValue::Unsigned(self.array.len() as u64)),
                Some(n) => self.array.get(n as usize - 1).cloned().ok_or_else(|| {
                    Error::protocol(ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX)
                }),
            },
            LIST => Ok(PropertyValue::List(self.list.clone())),
            REFERENCE => Ok(self.reference.clone()),
            _ => Ok(PropertyValue::Unsigned(1)),
        }
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        match (property, value) {
            (_, PropertyValue::Null) => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
            )),
            (LIST, PropertyValue::List(items)) => {
                self.list = items;
                Ok(())
            }
            (REFERENCE, value) => {
                self.reference = value;
                Ok(())
            }
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
            )),
        }
    }
}

fn vendor(
    object_type: ObjectType,
    name: &'static str,
    rows: &'static [PropertyMetadata],
) -> Vendor {
    Vendor {
        oid: ObjectIdentifier::new(object_type, 1).unwrap(),
        name,
        rows,
        array: vec![PropertyValue::Real(1.0), PropertyValue::Real(2.0)],
        list: vec![PropertyValue::Unsigned(1)],
        reference: PropertyValue::ApplicationData(Vec::new()),
    }
}

fn custom() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::from_raw(600), 1).unwrap()
}

fn db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(7, 0).unwrap(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(vendor(ObjectType::from_raw(600), "vendor", ROWS)))
        .unwrap();
    db.add(Box::new(vendor(
        ObjectType::CHANNEL,
        "channel",
        CHANNEL_ROWS,
    )))
    .unwrap();
    db
}

fn read(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack)?;
    Ok(ReadPropertyACK::decode(&ack).unwrap().property_value)
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn add_list_element(
    db: &mut ObjectDatabase,
    property: PropertyIdentifier,
    elements: &[u8],
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(&mut encoded, 0, &custom());
    bacnet_encoding::primitives::encode_ctx_enumerated(&mut encoded, 1, property.to_raw());
    encoded.extend_from_slice(&[0x3e]);
    encoded.extend_from_slice(elements);
    encoded.extend_from_slice(&[0x3f]);
    handle_add_list_element(db, &encoded)
}

fn is_error(result: &Result<impl std::fmt::Debug, Error>, code: ErrorCode) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { code: c, .. })) if *c == code)
}

#[test]
fn a_rows_array_type_admits_an_index_and_its_list_type_refuses_one() {
    let db = db();
    assert_eq!(read(&db, custom(), ARRAY, Some(0)).unwrap(), [0x21, 2]);
    assert_eq!(
        read(&db, custom(), ARRAY, Some(2)).unwrap(),
        [0x44, 0x40, 0, 0, 0]
    );
    let indexed_list = read(&db, custom(), LIST, Some(1));
    assert!(
        is_error(&indexed_list, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        "{indexed_list:?}"
    );
}

#[test]
fn the_list_services_edit_a_list_the_row_types_and_refuse_its_array() {
    let mut db = db();
    add_list_element(&mut db, LIST, &[0x21, 5]).unwrap();
    assert_eq!(read(&db, custom(), LIST, None).unwrap(), [0x21, 1, 0x21, 5]);
    let array = add_list_element(&mut db, ARRAY, &[0x44, 0, 0, 0, 0]);
    assert!(
        is_error(&array, ErrorCode::PROPERTY_IS_NOT_A_LIST),
        "{array:?}"
    );
}

#[test]
fn a_null_stands_refused_only_where_the_rows_datatype_admits_one() {
    let mut db = db();
    let null = [0x00];
    // The datatype admits NULL, so the object's refusal stands: a vendor
    // BACnetOptionalUnsigned, the standard Priority_Array's
    // BACnetPriorityValue and a Channel's BACnetChannelValue.
    for (oid, property) in [
        (custom(), NULLABLE),
        (custom(), PropertyIdentifier::PRIORITY_ARRAY),
        (
            ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
        ),
    ] {
        let result = write(&mut db, oid, property, &null);
        assert!(
            is_error(&result, ErrorCode::INVALID_DATA_TYPE),
            "{oid:?} {property:?}: {result:?}"
        );
    }
    // No NULL in the datatype, or no datatype stated: an unchanged success,
    // as before the rows carried datatypes.
    for property in [PLAIN, UNTYPED] {
        write(&mut db, custom(), property, &null)
            .unwrap_or_else(|error| panic!("{property:?}: {error:?}"));
    }
}

#[test]
fn a_custom_reference_keeps_the_local_device_it_names() {
    let mut db = db();
    let mut bytes = BytesMut::new();
    encode_device_object_property_reference(
        &mut bytes,
        &BACnetDeviceObjectPropertyReference {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap()),
        },
    );
    write(&mut db, custom(), REFERENCE, &bytes).unwrap();
    assert_eq!(
        read(&db, custom(), REFERENCE, None).unwrap(),
        bytes.to_vec()
    );
}
