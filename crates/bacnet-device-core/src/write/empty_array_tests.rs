//! An array written whole with no octets (#2305): an array of no elements,
//! which reaches its object like any other write (Clause 15.9.1.1.3). The
//! object answers by its own rules: a read-only array refuses the write for
//! access, an array that can shrink to nothing takes it, and one whose size
//! is fixed or has a floor refuses it as out of range. Zero octets for one
//! value, or for one element of an array, stay undecodable.
use super::decode_write_property_value;
use crate::write::{Applied, WpmOutcome, write_property, write_property_multiple};
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::property_metadata::{
    Datatype, PropertyConformance, PropertyMetadata, PropertyWriteCapability, ValueType,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// A vendor array of Unsigned on [`Resizable`], of any size.
const VENDOR_ARRAY: P = P::from_raw(600);

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn resizable() -> ObjectIdentifier {
    oid(ObjectType::from_raw(130), 1)
}

/// A proprietary object whose one array takes a whole write of any size,
/// none included.
struct Resizable(Vec<u64>);

impl BACnetObject for Resizable {
    fn object_identifier(&self) -> ObjectIdentifier {
        resizable()
    }
    fn object_name(&self) -> &str {
        "resizable"
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        match (property, index) {
            (VENDOR_ARRAY, None) => Ok(PropertyValue::List(
                self.0
                    .iter()
                    .copied()
                    .map(PropertyValue::Unsigned)
                    .collect(),
            )),
            (VENDOR_ARRAY, Some(0)) => Ok(PropertyValue::Unsigned(self.0.len() as u64)),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        property: P,
        index: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        let wrong_type = || Error::protocol(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE);
        let elements = match (property, index, value) {
            (VENDOR_ARRAY, None, PropertyValue::List(elements)) => elements,
            (VENDOR_ARRAY, None, single) => vec![single],
            _ => return Err(wrong_type()),
        };
        self.0 = elements
            .into_iter()
            .map(|element| match element {
                PropertyValue::Unsigned(n) => Ok(n),
                _ => Err(wrong_type()),
            })
            .collect::<Result<_, _>>()?;
        Ok(())
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Owned(vec![
            PropertyMetadata::new(
                VENDOR_ARRAY,
                PropertyConformance::Optional,
                None,
                PropertyWriteCapability::Always,
            )
            .with_value_type(ValueType::array(Datatype::Unsigned)),
        ])
    }
}

/// CMD-1 (a read-only Action), AI-1 (Event_Message_Texts_Config, three
/// elements always), MSV-1 (three states), CH-1 (no members, one control
/// group) and the [`Resizable`] array holding two elements.
fn database() -> ObjectDatabase {
    let identity = DeviceIdentity::new(1234, 0).unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 1).unwrap();
    channel.set_control_groups(vec![7]).unwrap();
    build_database_with_extra(
        &identity,
        vec![
            Box::new(CommandObject::new(1, "CMD-1").unwrap()),
            Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
            Box::new(channel),
            Box::new(Resizable(vec![1, 2])),
        ],
    )
    .unwrap()
}

/// What an empty write answers, or `None` for success.
type Answer = Option<(ErrorClass, ErrorCode)>;

const ACCEPTED: Answer = None;
const ACCESS_DENIED: Answer = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
const OUT_OF_RANGE: Answer = Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE));
const UNDECODABLE: Answer = Some((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING));

/// Each empty write and its answer: one per behaviour.
fn cases() -> Vec<(ObjectIdentifier, P, Option<u32>, Answer)> {
    vec![
        // A read-only array: refused for access, as a non-empty write is.
        (oid(ObjectType::COMMAND, 1), P::ACTION, None, ACCESS_DENIED),
        // An array that can shrink to nothing.
        (resizable(), VENDOR_ARRAY, None, ACCEPTED),
        // An array with one delay per member, and the Channel has none.
        (
            oid(ObjectType::CHANNEL, 1),
            P::EXECUTION_DELAY,
            None,
            ACCEPTED,
        ),
        // A fixed size, and two sizes with a floor of one.
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            OUT_OF_RANGE,
        ),
        (
            oid(ObjectType::CHANNEL, 1),
            P::CONTROL_GROUPS,
            None,
            OUT_OF_RANGE,
        ),
        (
            oid(ObjectType::MULTI_STATE_VALUE, 1),
            P::STATE_TEXT,
            None,
            OUT_OF_RANGE,
        ),
        // One value, and one element of an array.
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::DESCRIPTION,
            None,
            UNDECODABLE,
        ),
        (
            oid(ObjectType::CHANNEL, 1),
            P::CONTROL_GROUPS,
            Some(1),
            UNDECODABLE,
        ),
        (
            oid(ObjectType::CHANNEL, 1),
            P::CONTROL_GROUPS,
            Some(0),
            UNDECODABLE,
        ),
    ]
}

fn answer(error: &Error) -> Answer {
    match error.remote() {
        Some(RemoteError::Error { class, code }) => Some((class, code)),
        other => panic!("{other:?}"),
    }
}

fn size(db: &ObjectDatabase, object: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(property, Some(0))
        .unwrap()
}

#[test]
fn the_decoder_takes_no_octets_as_an_empty_array_written_whole() {
    let decode =
        |value_type, index| decode_write_property_value(VENDOR_ARRAY, value_type, index, &[]);
    let empty = PropertyValue::List(Vec::new());
    for whole in [
        ValueType::array(Datatype::Unsigned),
        ValueType::fixed_array(Datatype::Unsigned, 3),
        ValueType::list(Datatype::Unsigned),
    ] {
        assert_eq!(decode(Some(whole), None).unwrap(), empty, "{whole:?}");
    }
    for (value_type, index) in [
        (Some(ValueType::single(Datatype::Unsigned)), None),
        (None, None),
        (Some(ValueType::array(Datatype::Unsigned)), Some(0)),
        (Some(ValueType::array(Datatype::Unsigned)), Some(1)),
        (Some(ValueType::fixed_array(Datatype::Unsigned, 3)), Some(2)),
    ] {
        let error = decode(value_type, index).unwrap_err();
        assert_eq!(answer(&error), UNDECODABLE, "{value_type:?} {index:?}");
    }
    // An array of one element keeps the answer it had: the element alone.
    assert_eq!(
        decode_write_property_value(
            VENDOR_ARRAY,
            Some(ValueType::array(Datatype::Unsigned)),
            None,
            &[0x21, 0x05]
        )
        .unwrap(),
        PropertyValue::Unsigned(5)
    );
}

#[test]
fn write_property_of_an_empty_array_reaches_the_object() {
    let mut db = database();
    for (object, property, index, expected) in cases() {
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: index,
            property_value: Vec::new(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let result = write_property(&mut db, &body, None, None, None);
        match result {
            Ok((_, applied)) => {
                assert_eq!(expected, ACCEPTED, "{object:?} {property:?} {index:?}");
                assert_eq!(applied, Applied::Written);
            }
            Err(error) => assert_eq!(
                answer(&error),
                expected,
                "{object:?} {property:?} {index:?}"
            ),
        }
    }
    assert_eq!(
        size(&db, resizable(), VENDOR_ARRAY),
        PropertyValue::Unsigned(0)
    );
    // The refused writes changed nothing.
    let sizes = [
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            3,
        ),
        (oid(ObjectType::CHANNEL, 1), P::CONTROL_GROUPS, 1),
        (oid(ObjectType::MULTI_STATE_VALUE, 1), P::STATE_TEXT, 3),
    ];
    for (object, property, expected) in sizes {
        assert_eq!(
            size(&db, object, property),
            PropertyValue::Unsigned(expected),
            "{object:?} {property:?}"
        );
    }
}

#[test]
fn write_property_multiple_of_an_empty_array_reaches_the_object() {
    for (object, property, index, expected) in cases() {
        let mut db = database();
        // The resizable array first, then the case: a failure names the
        // case and keeps the write before it.
        let attempt = |object, property, index| WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: property,
                property_array_index: index,
                value: Vec::new(),
                priority: None,
            }],
        };
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![
                attempt(resizable(), VENDOR_ARRAY, None),
                attempt(object, property, index),
            ],
        }
        .encode(&mut body)
        .unwrap();
        match write_property_multiple(&mut db, &body, None, None, None, None) {
            WpmOutcome::Success { .. } => {
                assert_eq!(expected, ACCEPTED, "{object:?} {property:?} {index:?}");
            }
            WpmOutcome::Error {
                error,
                first_failed_write_attempt: failed,
                ..
            } => {
                assert_eq!(
                    answer(&error),
                    expected,
                    "{object:?} {property:?} {index:?}"
                );
                assert_eq!(
                    (failed.object_identifier, failed.property_identifier),
                    (object, property.to_raw())
                );
                assert_eq!(failed.property_array_index, index);
            }
            WpmOutcome::Reject { reason } => panic!("{object:?} {property:?}: {reason:?}"),
        }
        assert_eq!(
            size(&db, resizable(), VENDOR_ARRAY),
            PropertyValue::Unsigned(0)
        );
    }
}
